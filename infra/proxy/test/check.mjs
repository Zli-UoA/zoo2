import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
const config = JSON.parse(execFileSync('docker', ['compose', '--project-directory', '.',
  '-f', 'infra/compose.stg.yaml', 'config', '--format', 'json'], { encoding: 'utf8' }));
for (const [name, service] of Object.entries(config.services)) {
  if (name !== 'proxy') assert.equal(service.ports, undefined, `${name} must not publish ports`);
}
assert.equal(config.services.frontend.build.args.VITE_API_BASE_URL, '');
assert.equal(config.services.backend.environment.ENVIRONMENT, 'production');
assert.equal(config.services.backend.environment.DEV_AUTH_ENABLED, 'false');
assert.equal(config.networks.backend.internal, true);
const base = process.env.PROXY_TEST_URL || 'http://127.0.0.1:18080';
const request = (path, options = {}) => fetch(`${base}${path}`, options);
const spoofed = { Authorization: 'Bearer attacker', 'X-Dev-User': 'attacker' };
const baseline = await (await request('/api/test-baseline', {
  headers: { Cookie: 'session=valid' },
})).json();
assert.equal(await (await request('/')).text(), 'frontend');
assert.equal((await request('/api/channels', { headers: spoofed })).status, 401);
assert.equal((await request('/auth/verify', { headers: { Cookie: 'session=valid' } })).status, 418);
assert.equal((await request('/auth/verify/')).status, 418);
assert.equal((await request('/api/channels', { headers: { Cookie: 'session=unavailable' } })).status, 503);
assert.equal((await request('/api/channels', { method: 'POST', headers: {
  Cookie: 'session=valid', Origin: 'https://attacker.example',
  'X-Forwarded-Method': 'GET', 'X-Forwarded-Host': 'attacker.example',
} })).status, 403);
const response = await request('/api/channels?limit=1', { method: 'POST',
  headers: { ...spoofed, Cookie: 'session=valid', Origin: base,
    'X-Forwarded-Proto': 'https', 'X-Forwarded-Host': 'attacker.example' }, body: 'payload' });
assert.equal(response.status, 200);
assert.equal(response.headers.get('authorization'), null);
assert.equal(response.headers.get('cache-control'), 'private, no-cache');
assert.deepEqual(await response.json(), { method: 'POST', url: '/api/channels?limit=1',
  body: 'payload', authorization: 'Bearer verified-token', devUser: null,
  backendRequests: baseline.backendRequests + 1 });
const stream = await request('/api/shapes/messages?offset=-1&live=true', {
  headers: { Cookie: 'session=valid' },
});
assert.equal(stream.headers.get('electric-offset'), '42_0');
assert.equal(stream.headers.get('electric-handle'), 'test-handle');
assert.equal(stream.headers.get('vary'), 'Authorization');
const reader = stream.body.getReader();
const first = await reader.read();
assert.equal(new TextDecoder().decode(first.value), '[{"first":true},');
assert.equal(first.done, false);
let tail = '';
while (true) {
  const chunk = await reader.read();
  if (chunk.done) break;
  tail += new TextDecoder().decode(chunk.value);
}
assert.equal(tail, '{"last":true}]');
console.log('PASS: routing, fail-closed auth, spoof stripping, CSRF metadata, body forwarding, streaming and Electric headers');
