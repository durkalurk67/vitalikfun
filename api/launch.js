// POST /api/launch
// { creator, name, symbol, uri, houseBps, shares: [{ address, bps }], buyback?, burnBps, homeTable, devBuySol? }
//
// Returns the transactions the creator approves together in one wallet prompt. In order they:
//   1) create the coin on pump.fun (with an optional first-block dev buy)
//   2) register it with Housepad and open its casino-stake address
//   3) turn on pump.fun creator-fee sharing and lock the split:
//        Housepad's share + the coin's casino stake + the creator's wallets
//      (update_fee_shares_v2 can only run once, so the split is permanent)
// Steps are packed into as few transactions as fit. The server signs only with the brand-new mint key;
// the creator's wallet signs and pays for everything.
const { Keypair } = require('@solana/web3.js');
const { env, handler, readJson, pubkey, HttpError } = require('./_lib.js');
const { PUMP_SDK, compute, tryBuild, shareholders, configIx, sharesIx, parseDevBuySol, quoteDevBuy, pumpPortalCreateTx } = require('./_build.js');
const { NUM_TABLES, feesPda, registerCoinIx, houseIsOpen } = require('./_housepad.js');

const flat = (a) => [].concat(...a.map((x) => (Array.isArray(x) ? x : [x])));
const bytes = (s) => Buffer.byteLength(s, 'utf8');

// Greedily packs ordered steps into transactions that fit under the size limit.
function pack(payer, steps, blockhash, units) {
  const txs = []; let cur = [];
  for (const step of steps) {
    const trial = cur.concat([step]);
    if (tryBuild(payer, flat([compute(units), ...trial.map((s) => s.ixs)]), blockhash)) { cur = trial; continue; }
    if (!cur.length) throw new HttpError(400, `The ${step.label} step is too large for one transaction. Try a shorter name.`);
    txs.push(cur); cur = [step];
    if (!tryBuild(payer, flat([compute(units), ...cur.map((s) => s.ixs)]), blockhash)) throw new HttpError(400, `The ${step.label} step is too large for one transaction. Try a shorter name.`);
  }
  if (cur.length) txs.push(cur);
  return txs.map((group) => ({ labels: group.map((s) => s.label), tx: tryBuild(payer, flat([compute(units), ...group.map((s) => s.ixs)]), blockhash) }));
}

function signIfNeeded(tx, kp) {
  const n = tx.message.header.numRequiredSignatures;
  if (tx.message.staticAccountKeys.slice(0, n).some((k) => k.equals(kp.publicKey))) tx.sign([kp]);
}

module.exports = handler(async (req) => {
  if (!PUMP_SDK) throw new HttpError(500, 'pump.fun SDK did not load.');
  const { connection, platform, platformBps, houseMinBps, programId } = env();
  const b = await readJson(req, 20_000);

  const creator = pubkey(b.creator, 'Your wallet');
  const name = String(b.name ?? '').trim(), symbol = String(b.symbol ?? '').trim().toUpperCase(), uri = String(b.uri ?? '').trim();
  if (!name || bytes(name) > 32) throw new HttpError(400, 'Name must be 1–32 characters.');
  if (!/^[A-Z0-9]{1,13}$/.test(symbol)) throw new HttpError(400, 'Ticker must be 1–13 letters or numbers.');
  if (!/^https:\/\//.test(uri) || bytes(uri) > 200) throw new HttpError(400, 'Metadata link is missing. Upload the image first.');
  const houseBps = Number(b.houseBps);
  if (!Number.isInteger(houseBps) || houseBps < houseMinBps || houseBps > 10_000 - platformBps - 100) {
    throw new HttpError(400, `The casino stake must be between ${houseMinBps / 100}% and ${(10_000 - platformBps - 100) / 100}% of creator fees.`);
  }
  const burnBps = Number(b.burnBps ?? 5000);
  if (!Number.isInteger(burnBps) || burnBps < 0 || burnBps > 10_000) throw new HttpError(400, 'Burn share must be 0–100%.');
  const homeTable = Number(b.homeTable ?? 0);
  if (!Number.isInteger(homeTable) || homeTable < 0 || homeTable >= NUM_TABLES) throw new HttpError(400, 'Pick a home table.');
  const buyback = b.buyback ? pubkey(b.buyback, 'Buyback wallet') : creator;
  const devBuySol = parseDevBuySol(b.devBuySol);

  const house = await houseIsOpen(connection, programId);
  if (!house.open) throw new HttpError(400, 'The casino isn’t open yet, so coins can’t launch into it. Try again once it’s live.');
  if (house.paused) throw new HttpError(400, 'The casino is paused right now, so new coins can’t launch into it.');

  const mint = Keypair.generate();
  const intake = feesPda(programId, mint.publicKey);
  const holders = shareholders({ shares: b.shares, platform, platformBps, intake, houseBps });
  const register = registerCoinIx({ programId, creator, mint: mint.publicKey, buyback, name, symbol, uri, burnBps, homeTable });
  const config = await configIx(creator, mint.publicKey);
  const shares = await sharesIx(creator, mint.publicKey, holders);
  const { blockhash, lastValidBlockHeight } = await connection.getLatestBlockhash('confirmed');

  let groups, devBuy = null, first = null;
  if (devBuySol > 0) {
    // PumpPortal builds create + dev buy compactly as its own transaction; everything else packs after it.
    first = await pumpPortalCreateTx({ creator, mint: mint.publicKey, name, symbol, uri, sol: devBuySol });
    const q = await quoteDevBuy(connection, devBuySol).catch(() => null);
    devBuy = { sol: devBuySol, tokens: q ? q.tokens.toString() : null, pct: q ? q.pct : null };
    groups = pack(creator, [
      { label: 'register', ixs: [register] }, { label: 'fee-sharing', ixs: config }, { label: 'fee-split', ixs: shares },
    ], blockhash, 400_000);
  } else {
    const create = await PUMP_SDK.createV2Instruction({ mint: mint.publicKey, name, symbol, uri, creator, user: creator, mayhemMode: false, cashback: false });
    groups = pack(creator, [
      { label: 'create', ixs: create }, { label: 'register', ixs: [register] },
      { label: 'fee-sharing', ixs: config }, { label: 'fee-split', ixs: shares },
    ], blockhash, 400_000);
  }
  const txs = (first ? [first] : []).concat(groups.map((g) => g.tx));
  const steps = (first ? [['create', 'dev-buy']] : []).concat(groups.map((g) => g.labels));
  txs.forEach((tx) => signIfNeeded(tx, mint));

  // Dry-run the first transaction so the creator never signs something that would fail on-chain.
  const sim = await connection.simulateTransaction(txs[0], { sigVerify: false, replaceRecentBlockhash: false });
  if (sim.value.err) {
    const logs = (sim.value.logs || []).slice(-6).join(' | ');
    console.error('launch simulation failed', JSON.stringify(sim.value.err), logs);
    if (/insufficient lamports|insufficient funds/i.test(logs)) throw new HttpError(400, devBuySol > 0 ? `Your wallet needs more SOL: the ${devBuySol} SOL dev buy plus about 0.03 SOL in fees and deposits.` : 'Your wallet needs a little more SOL (about 0.03) to cover the launch.');
    if (/slippage|TooMuchSolRequired|exceeds/i.test(logs)) throw new HttpError(400, 'The dev buy price moved past the 5% allowance. Please try again.');
    throw new HttpError(502, `The launch would fail on-chain right now. Details: ${logs.slice(-240) || JSON.stringify(sim.value.err)}`);
  }

  return {
    transactions: txs.map((t) => Buffer.from(t.serialize()).toString('base64')),
    steps, mint: mint.publicKey.toBase58(), intake: intake.toBase58(), lastValidBlockHeight, devBuy,
    // Lets this page re-send the Housepad registration if that one transaction doesn't land.
    // The mint key can only sign as this coin's address; it controls no funds.
    mintKey: Buffer.from(mint.secretKey).toString('base64'),
    shareholders: holders.map((s) => ({ address: s.address.toBase58(), bps: s.shareBps })),
  };
});
