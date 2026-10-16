import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
const config = JSON.parse(execFileSync('docker', ['compose', '--project-directory', '.',
  '-f', 'infra/compose.stg.yaml', 'config', '--format', 'json'], { encoding: 'utf8' }));
for (const [name, service] of Object.entries(config.services)) {
  if (name !== 'proxy') assert.equal(service.ports, undefined, `${name} must not publish ports`);
}
assert.equal(config.services.proxy.ports.length, 1);
assert.equal(config.services.proxy.ports[0].host_ip, '127.0.0.1');
assert.equal(config.services.proxy.ports[0].target, 8080);
assert.equal(config.services.proxy.ports[0].published, String(process.env.PROXY_PORT || 18080));
assert.equal(config.services.frontend.build.args.VITE_API_BASE_URL, '');
assert.equal(config.services.backend.environment.ENVIRONMENT, 'production');
assert.equal(config.services.backend.environment.DEV_AUTH_ENABLED, 'false');
assert.equal(config.networks.backend.internal, true);
const base = process.env.PROXY_TEST_URL || 'http://127.0.0.1:18080';
const request = (path, options = {}) => fetch(`${base}${path}`, options);
assert.equal(await (await request('/')).text(), 'frontend');
assert.equal(await (await request('/channels/general')).text(), 'frontend');
assert.equal((await request('/api/channels')).status, 401);
assert.equal((await request('/api/channels', {
  headers: { 'X-Dev-User': 'attacker' },
})).status, 401);
const response = await request('/api/channels?limit=1', {
  method: 'POST',
  headers: {
    Authorization: 'Bearer valid-token',
    'X-Dev-User': 'attacker',
  },
  body: 'payload',
});
assert.equal(response.status, 200);
assert.equal(response.headers.get('cache-control'), 'private, no-cache');
assert.deepEqual(await response.json(), {
  method: 'POST',
  url: '/api/channels?limit=1',
  body: 'payload',
  authorization: 'Bearer valid-token',
  devUser: null,
});
const stream = await request('/api/shapes/messages?offset=-1&live=true', {
  headers: { Authorization: 'Bearer valid-token' },
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
console.log('PASS: routing, Bearer forwarding, X-Dev-User stripping, body forwarding, streaming and Electric headers');

