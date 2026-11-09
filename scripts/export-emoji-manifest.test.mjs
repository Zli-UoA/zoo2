import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, writeFile, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
test('export keeps identity and legacy UUID, hashes bytes, and rejects ambiguous identities', async () => {
  const root = await mkdtemp(join(tmpdir(), 'zoo-emoji-export-'));
  try {
    await mkdir(join(root, 'images'));
    const id = '11111111-1111-1111-1111-111111111111', bytes = Buffer.from('image-bytes');
    await writeFile(join(root, 'images', id), bytes);
    await writeFile(join(root, 'images', '22222222-2222-2222-2222-222222222222'), Buffer.from('old-image'));
    const rows = [{ name: 'cat', category: '', aliases: ['neko'], created_by: 'local-1', url: '/api/emoji-images/' + id, identities: [{ iss: 'https://auth.test', sub: 'alice' }] }];
    const source = join(root, 'rows.json'), output = join(root, 'manifest.json');
    await writeFile(source, JSON.stringify(rows));
    const run = () => spawnSync(process.execPath, ['scripts/export-emoji-manifest.mjs', source, join(root, 'images'), output], { encoding: 'utf8' });
    assert.equal(run().status, 0);
    const exported = JSON.parse(await readFile(output, 'utf8'));
    const result = exported[0];
    assert.equal(exported.length, 2);
    assert.equal(exported[1].name, undefined);
    assert.equal(exported[1].legacy_id, '22222222-2222-2222-2222-222222222222');
    assert.equal(result.subject, 'alice'); assert.equal(result.legacy_id, id);
    assert.equal(result.sha256, createHash('sha256').update(bytes).digest('hex'));
    await rm(output);
    rows[0].identities.push({ iss: 'https://auth.test', sub: 'other' });
    await writeFile(source, JSON.stringify(rows));
    assert.notEqual(run().status, 0);
    await assert.rejects(readFile(output));
  } finally { await rm(root, { recursive: true, force: true }); }
});
