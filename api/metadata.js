// POST /api/metadata { name, symbol, description, imageBase64, imageType, twitter?, telegram?, website? }
// Uploads the image + metadata to pump.fun's IPFS endpoint and returns the metadata URI.
const { handler, readJson, HttpError } = require('./_lib.js');

const TYPES = { 'image/png': 'png', 'image/jpeg': 'jpg', 'image/gif': 'gif', 'image/webp': 'webp' };
const clean = (v, max) => String(v ?? '').trim().slice(0, max);
const url = (v) => { const s = clean(v, 200); if (!s) return ''; try { const u = new URL(s); return /^https?:$/.test(u.protocol) ? u.toString() : ''; } catch { return ''; } };

module.exports = handler(async (req) => {
  const b = await readJson(req, 4_000_000);
  const name = clean(b.name, 32), symbol = clean(b.symbol, 13).toUpperCase(), description = clean(b.description, 500);
  if (!name || !symbol) throw new HttpError(400, 'Name and ticker are required.');
  const ext = TYPES[b.imageType];
  if (!ext || typeof b.imageBase64 !== 'string') throw new HttpError(400, 'Add a PNG, JPG, GIF or WebP image.');
  const bytes = Buffer.from(b.imageBase64, 'base64');
  if (bytes.length < 100 || bytes.length > 2_900_000) throw new HttpError(400, 'Image must be under about 2.8 MB.');

  const fd = new FormData();
  fd.append('file', new Blob([bytes], { type: b.imageType }), `token.${ext}`);
  fd.append('name', name); fd.append('symbol', symbol); fd.append('description', description);
  fd.append('twitter', url(b.twitter)); fd.append('telegram', url(b.telegram)); fd.append('website', url(b.website));
  fd.append('showName', 'true');
  const r = await fetch('https://pump.fun/api/ipfs', { method: 'POST', body: fd });
  if (!r.ok) throw new HttpError(502, `Image upload failed (${r.status}). Please try again.`);
  const j = await r.json();
  if (!j.metadataUri) throw new HttpError(502, 'Image upload returned no metadata link.');
  return { metadataUri: j.metadataUri, image: (j.metadata && j.metadata.image) || null };
});
