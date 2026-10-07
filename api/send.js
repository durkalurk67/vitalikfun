// POST /api/send { transaction }  — fallback for wallets that can sign but not send.
const { VersionedTransaction } = require('@solana/web3.js');
const { env, handler, readJson, HttpError } = require('./_lib.js');
module.exports = handler(async (req) => {
  const { connection } = env();
  const b = await readJson(req, 20_000);
  let tx; try { tx = VersionedTransaction.deserialize(Buffer.from(String(b.transaction), 'base64')); } catch { throw new HttpError(400, 'Invalid transaction.'); }
  const signature = await connection.sendRawTransaction(tx.serialize(), { skipPreflight: false, maxRetries: 3 });
  return { signature };
});
