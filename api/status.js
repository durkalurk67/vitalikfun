// GET /api/status?sig=... -> confirmation status, so the browser never needs its own RPC key.
const { env, handler, HttpError } = require('./_lib.js');
module.exports = handler(async (req) => {
  const { connection } = env();
  const sig = new URL(req.url, 'http://x').searchParams.get('sig') || '';
  if (!/^[1-9A-HJ-NP-Za-km-z]{60,100}$/.test(sig)) throw new HttpError(400, 'Invalid signature.');
  const { value } = await connection.getSignatureStatuses([sig], { searchTransactionHistory: true });
  const st = value[0];
  return { status: st && st.err ? 'failed' : (st && st.confirmationStatus) || 'pending', err: (st && st.err) || null };
});
