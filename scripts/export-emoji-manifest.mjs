import { readFile, writeFile, readdir } from 'node:fs/promises';
import { resolve, dirname, relative } from 'node:path';
import { createHash } from 'node:crypto';
const [rowsPath, imagesPath, outputPath] = process.argv.slice(2);
if (!rowsPath || !imagesPath || !outputPath) throw Error('Usage: node scripts/export-emoji-manifest.mjs rows.json image-directory manifest.json');
const rows = JSON.parse(await readFile(rowsPath, 'utf8'));
if (!Array.isArray(rows)) throw Error('Expected exported JSON array');
const manifest = [];
for (const row of rows) {
  const match = /^\/api\/emoji-images\/([a-f0-9-]{36})$/.exec(row.url);
  if (!match) throw Error(`Export image manually and prepare a manifest entry for external URL: ${row.name}`);
  const image = resolve(imagesPath, match[1]);
  const file = relative(dirname(resolve(outputPath)), image);
  if (file.startsWith('..')) throw Error('Images must be inside the manifest directory');
  const bytes = await readFile(image);
  if (!Array.isArray(row.identities) || row.identities.length > 1) throw Error(`Ambiguous AuthZ identities for ${row.name}`);
  const identity = row.identities[0];
  manifest.push({ name: row.name, category: row.category, aliases: row.aliases, image: file,
    sha256: createHash('sha256').update(bytes).digest('hex'), legacy_id: match[1], created_by: row.created_by,
    ...(identity ? { issuer: identity.iss, subject: identity.sub } : {}) });
}
const included = new Set(manifest.map(row => row.legacy_id));
for (const id of (await readdir(imagesPath)).sort()) {
  if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(id) || included.has(id)) continue;
  const path = resolve(imagesPath, id), image = relative(dirname(resolve(outputPath)), path);
  if (image.startsWith('..')) throw Error('Images must be inside the manifest directory');
  const bytes = await readFile(path);
  manifest.push({ image, legacy_id: id, sha256: createHash('sha256').update(bytes).digest('hex') });
}
await writeFile(outputPath, JSON.stringify(manifest, null, 2), { flag: 'wx', mode: 0o600 });
console.log(JSON.stringify({ exported: manifest.length, catalog_entries: rows.length, historical_images: manifest.length - rows.length }));
