// POST /api/split { creator, mint, houseBps, shares: [{ address, bps }] }
// Rebuilds the "turn on fee sharing + lock the split" step for a coin whose launch didn't finish.
// The creator signs it; no mint key is needed. pump.fun ignores the config step if it already ran,
// and only lets the split be set once, so this can't overwrite a split that's already locked.
const { PublicKey } = require('@solana/web3.js');
const { env, handler, readJson, pubkey, HttpError } = require('./_lib.js');
const { PUMP_SDK, compute, tryBuild, shareholders, configIx, sharesIx } = require('./_build.js');
const { feesPda } = require('./_housepad.js');

const flat = (a) => [].concat(...a.map((x) => (Array.isArray(x) ? x : [x])));

module.exports = handler(async (req) => {
  if (!PUMP_SDK) throw new HttpError(500, 'pump.fun SDK did not load.');
  const { connection, platform, platformBps, houseMinBps, programId } = env();
  const b = await readJson(req, 20_000);
  const creator = pubkey(b.creator, 'Your wallet');
  let mint; try { mint = new PublicKey(String(b.mint)); } catch { throw new HttpError(400, 'Invalid coin address.'); }
  const houseBps = Number(b.houseBps);
  if (!Number.isInteger(houseBps) || houseBps < houseMinBps || houseBps > 10_000 - platformBps - 100) throw new HttpError(400, 'Invalid casino stake.');
  const holders = shareholders({ shares: b.shares, platform, platformBps, intake: feesPda(programId, mint), houseBps });
  const { blockhash, lastValidBlockHeight } = await connection.getLatestBlockhash('confirmed');
  const tx = tryBuild(creator, flat([compute(400_000), await configIx(creator, mint), await sharesIx(creator, mint, holders)]), blockhash);
  if (!tx) throw new HttpError(400, 'The fee split is too large to fit in one transaction.');
  return { transaction: Buffer.from(tx.serialize()).toString('base64'), lastValidBlockHeight };
});
