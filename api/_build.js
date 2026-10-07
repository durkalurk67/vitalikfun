// Builds the pump.fun launch + locked fee split (adapted from the Shopi-fi launcher, CommonJS).
// Solana transactions max out at 1232 bytes, so instructions are packed into as few transactions
// as actually fit, measured rather than assumed.
const { ComputeBudgetProgram, TransactionMessage, VersionedTransaction } = require('@solana/web3.js');
const { NATIVE_MINT, TOKEN_PROGRAM_ID } = require('@solana/spl-token');
const pumpSdk = require('@pump-fun/pump-sdk');
const { HttpError, pubkey, MAX_SHAREHOLDERS } = require('./_lib.js');

const PUMP_SDK = pumpSdk.PUMP_SDK || (pumpSdk.default && pumpSdk.default.PUMP_SDK);
const OnlinePumpSdk = pumpSdk.OnlinePumpSdk || (pumpSdk.default && pumpSdk.default.OnlinePumpSdk);
const getBuyTokenAmountFromSolAmount = pumpSdk.getBuyTokenAmountFromSolAmount || (pumpSdk.default && pumpSdk.default.getBuyTokenAmountFromSolAmount);
const BN = require('bn.js');

const TOTAL_SUPPLY_BASE_UNITS = new BN('1000000000000000'); // 1B tokens, 6 decimals
const MAX_DEV_BUY_SOL = 1000;
const MAX_TX_BYTES = 1232;

function parseDevBuySol(v) {
  if (v === undefined || v === null || v === '') return 0;
  const n = Number(v);
  if (!Number.isFinite(n) || n < 0) throw new HttpError(400, 'Dev buy must be a positive SOL amount (or 0).');
  if (n > MAX_DEV_BUY_SOL) throw new HttpError(400, `Dev buy can't exceed ${MAX_DEV_BUY_SOL} SOL.`);
  return Math.floor(n * 1e9) / 1e9;
}

async function quoteDevBuy(connection, sol) {
  if (!OnlinePumpSdk || !getBuyTokenAmountFromSolAmount) throw new HttpError(500, 'pump.fun SDK is missing the buy helpers.');
  const online = new OnlinePumpSdk(connection);
  const [global, feeConfig] = await Promise.all([online.fetchGlobal(), online.fetchFeeConfig().catch(() => null)]);
  const lamports = new BN(Math.round(sol * 1e9).toString());
  const tokens = getBuyTokenAmountFromSolAmount({ global, feeConfig, mintSupply: null, bondingCurve: null, amount: lamports });
  const pct = tokens.mul(new BN(10000)).div(TOTAL_SUPPLY_BASE_UNITS).toNumber() / 100;
  return { lamports, tokens, pct };
}

function compute(units) {
  return [ComputeBudgetProgram.setComputeUnitLimit({ units }), ComputeBudgetProgram.setComputeUnitPrice({ microLamports: 50_000 })];
}

// Returns a VersionedTransaction if it fits, otherwise null.
function tryBuild(payer, ixs, blockhash) {
  try {
    const msg = new TransactionMessage({ payerKey: payer, recentBlockhash: blockhash, instructions: ixs }).compileToV0Message();
    const tx = new VersionedTransaction(msg);
    return tx.serialize().length <= MAX_TX_BYTES ? tx : null;
  } catch { return null; }
}

// Full shareholder list: Housepad, the coin's casino stake, then the creator's wallets.
function shareholders({ shares, platform, platformBps, intake, houseBps }) {
  const creatorBps = 10_000 - platformBps - houseBps;
  const list = Array.isArray(shares) ? shares.filter((s) => s && s.address) : [];
  if (!list.length) throw new HttpError(400, 'Add at least one wallet to receive the creator share.');
  const fixed = (platformBps > 0 ? 1 : 0) + 1;
  if (list.length > MAX_SHAREHOLDERS - fixed) throw new HttpError(400, `Too many fee wallets. You can add up to ${MAX_SHAREHOLDERS - fixed}.`);
  const seen = new Set([platform.toBase58(), intake.toBase58()]);
  const out = list.map((s, i) => {
    const address = pubkey(s.address, `Fee wallet ${i + 1}`);
    const key = address.toBase58();
    if (seen.has(key)) throw new HttpError(400, `Fee wallet ${i + 1} is listed twice (or is the Housepad wallet).`);
    seen.add(key);
    const bps = Number(s.bps);
    if (!Number.isInteger(bps) || bps <= 0) throw new HttpError(400, `Fee wallet ${i + 1} needs a share above 0%.`);
    return { address, shareBps: bps };
  });
  if (out.reduce((n, s) => n + s.shareBps, 0) !== creatorBps) throw new HttpError(400, `Creator wallets must add up to ${creatorBps / 100}%.`);
  out.unshift({ address: intake, shareBps: houseBps });
  if (platformBps > 0) out.unshift({ address: platform, shareBps: platformBps });
  return out;
}

async function configIx(creator, mint) { return PUMP_SDK.createFeeSharingConfig({ creator, mint, pool: null }); }
async function sharesIx(creator, mint, newShareholders) {
  return PUMP_SDK.updateFeeSharesV2({
    authority: creator, mint, currentShareholders: [creator], newShareholders,
    quoteMint: NATIVE_MINT, quoteTokenProgram: TOKEN_PROGRAM_ID,
  });
}

// Coin creation + first-block dev buy, built compactly by PumpPortal (https://pumpportal.fun/creation).
async function pumpPortalCreateTx({ creator, mint, name, symbol, uri, sol }) {
  const r = await fetch('https://pumpportal.fun/api/trade-local', {
    method: 'POST', headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      publicKey: creator.toBase58(), action: 'create', tokenMetadata: { name, symbol, uri }, mint: mint.toBase58(),
      denominatedInSol: 'true', amount: sol, slippage: 5, priorityFee: 0.0005, pool: 'pump',
    }),
    signal: AbortSignal.timeout(15000),
  });
  if (r.status !== 200) {
    const text = (await r.text().catch(() => '')).slice(0, 200);
    throw new HttpError(502, `Couldn't prepare the dev-buy launch right now (${r.status}${text ? ': ' + text : ''}). Try again, or launch without a dev buy.`);
  }
  return VersionedTransaction.deserialize(new Uint8Array(await r.arrayBuffer()));
}

module.exports = { PUMP_SDK, compute, tryBuild, shareholders, configIx, sharesIx, parseDevBuySol, quoteDevBuy, pumpPortalCreateTx };
