import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';

// Run inside the real frontend image with generic HOST/PORT overridden, as can
// happen when deployment variables are shared with the backend.
const server = spawn(process.execPath, ['server/index.mjs'], { stdio: 'inherit' });
try {
  let response;
  for (let attempt = 0; attempt < 40; attempt++) {
    if (server.exitCode !== null) throw new Error(`Frontend exited: ${server.exitCode}`);
    try {
      response = await fetch('http://127.0.0.1:3000/');
      break;
    } catch {
      await new Promise(resolve => setTimeout(resolve, 250));
    }
  }
  assert.equal(response?.status, 200, 'Frontend must serve healthcheck/proxy port 3000');
  assert.equal(process.env.NITRO_PORT, '3000');
  assert.equal(process.env.NITRO_HOST, '0.0.0.0');
  console.log('PASS: frontend remains on 0.0.0.0:3000 with shared HOST/PORT overrides');
} finally {
  server.kill();
}
