// Shared helpers for the Housepad launch API (Vercel serverless functions, CommonJS).
const { Connection, PublicKey } = require('@solana/web3.js');

const MAX_SHAREHOLDERS = 10; // pump.fun limit, includes Housepad and the coin's casino stake
const DEFAULT_PROGRAM_ID = '9HxeupB5dzsGzG9xzrohMpPYeyNjbXHKCHVVRTBtEYQY';
const DEFAULT_PLATFORM = 'CBdVbUjVuhMSDH39DoYrDg3TCqXFa8cWQrm68chBmvb8';

class HttpError extends Error { constructor(status, message) { super(message); this.status = status; } }

function bpsEnv(name, fallback, min, max) {
  const v = Number(process.env[name] ?? fallback);
  if (!Number.isInteger(v) || v < min || v > max) throw new HttpError(500, `${name} must be a whole number from ${min} to ${max}.`);
  return v;
}

function env() {
  const rpc = process.env.SOLANA_RPC_URL;
  if (!rpc) throw new HttpError(500, 'The launch service is not configured yet (SOLANA_RPC_URL is missing).');
  const url = rpc.trim().replace(/^['"]|['"]$/g, '');
  if (/^SOLANA_RPC_URL=/i.test(url)) throw new HttpError(500, 'SOLANA_RPC_URL value starts with "SOLANA_RPC_URL=". Paste only the https:// address.');
  if (/^wss?:\/\//i.test(url)) throw new HttpError(500, 'SOLANA_RPC_URL is a websocket (wss://) address. Use the https:// one.');
  if (!/^https?:\/\//i.test(url)) throw new HttpError(500, 'SOLANA_RPC_URL must be a full https:// address.');
  let connection;
  try { connection = new Connection(url, 'confirmed'); } catch (e) { throw new HttpError(500, `SOLANA_RPC_URL was rejected: ${e.message}`); }
  let platform, programId;
  try { platform = new PublicKey(String(process.env.PLATFORM_WALLET || DEFAULT_PLATFORM).trim()); } catch { throw new HttpError(500, 'PLATFORM_WALLET is not a valid Solana address.'); }
  try { programId = new PublicKey(String(process.env.HOUSEPAD_PROGRAM_ID || DEFAULT_PROGRAM_ID).trim()); } catch { throw new HttpError(500, 'HOUSEPAD_PROGRAM_ID is not a valid address.'); }
  const platformBps = bpsEnv('PLATFORM_FEE_BPS', 1000, 0, 5000);
  const houseMinBps = bpsEnv('HOUSE_MIN_BPS', 2000, 0, 9000);
  if (platformBps + houseMinBps > 9900) throw new HttpError(500, 'PLATFORM_FEE_BPS + HOUSE_MIN_BPS leaves nothing for creators.');
  return { connection, platform, platformBps, houseMinBps, programId };
}

// Wallet addresses typed by people must be real wallets (on the ed25519 curve), not program addresses.
function pubkey(v, label) {
  try { const k = new PublicKey(String(v).trim()); if (!PublicKey.isOnCurve(k.toBytes())) throw 0; return k; }
  catch { throw new HttpError(400, `${label} is not a valid Solana wallet address.`); }
}

async function readJson(req, limitBytes) {
  if (req.body && typeof req.body === 'object' && !Buffer.isBuffer(req.body)) return req.body;
  if (typeof req.body === 'string') { try { return JSON.parse(req.body || '{}'); } catch { throw new HttpError(400, 'Invalid JSON.'); } }
  let size = 0; const chunks = [];
  for await (const c of req) { size += c.length; if (size > limitBytes) throw new HttpError(413, 'Request is too large.'); chunks.push(c); }
  try { return JSON.parse(Buffer.concat(chunks).toString('utf8') || '{}'); } catch { throw new HttpError(400, 'Invalid JSON.'); }
}

function send(res, status, body) {
  res.statusCode = status;
  res.setHeader('Content-Type', 'application/json; charset=utf-8');
  res.setHeader('Cache-Control', 'no-store');
  res.end(JSON.stringify(body));
}

function handler(fn) {
  return async (req, res) => {
    if (req.method !== 'POST' && req.method !== 'GET') { send(res, 405, { error: 'Method not allowed' }); return; }
    try { send(res, 200, await fn(req)); }
    catch (e) {
      const status = e instanceof HttpError ? e.status : 500;
      if (status === 500) console.error(e);
      send(res, status, { error: e instanceof HttpError ? e.message : `Something went wrong: ${String((e && e.message) || e).slice(0, 200)}` });
    }
  };
}

module.exports = { MAX_SHAREHOLDERS, HttpError, env, pubkey, readJson, send, handler };
