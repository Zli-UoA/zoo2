import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createHash } from 'node:crypto';
import { globSync, mkdtempSync, mkdirSync, readFileSync, writeFileSync, rmSync, statSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';

const action = readFileSync(new URL('../actions/crypto-wasm/action.yaml', import.meta.url), 'utf8');
const patterns = [...action.match(/hashFiles\((.*?)\)/)[1].matchAll(/'([^']+)'/g)].map(m => m[1]);
const hash = cwd => {
  const files = [...new Set(patterns.flatMap(pattern => globSync(pattern, { cwd })))].filter(path => statSync(join(cwd, path)).isFile()).sort();
  const digest = createHash('sha256');
  for (const file of files) digest.update(createHash('sha256').update(readFileSync(join(cwd, file))).digest());
  return digest.digest('hex');
};

test('Wasm cache changes for source, dependencies, compiler and build settings, but not frontend or tests', () => {
  const dir = mkdtempSync(join(tmpdir(), 'zoo-wasm-key-'));
  const write = (path, data) => { mkdirSync(dirname(join(dir, path)), { recursive: true }); writeFileSync(join(dir, path), data); };
  try {
    write('crates/zoo-crypto/src/lib.rs', 'source');
    const initial = hash(dir);
    for (const path of ['frontend/src/app.tsx', 'crates/zoo-crypto/tests/vectors.rs', 'crates/zoo-crypto-wasm/target/old.wasm']) {
      write(path, 'unrelated'); assert.equal(hash(dir), initial, path);
    }
    for (const path of ['crates/zoo-crypto/src/lib.rs', 'crates/zoo-crypto/Cargo.toml', 'crates/zoo-crypto-wasm/Cargo.lock', 'crates/zoo-crypto-wasm/src/lib.rs', 'crates/zoo-crypto-wasm/build.sh', 'crates/zoo-crypto-wasm/build.rs', 'rust-toolchain.toml', '.cargo/config.toml', '.github/actions/crypto-wasm/action.yaml']) {
      const before = hash(dir); write(path, `new input: ${path}`); assert.notEqual(hash(dir), before, path);
    }
  } finally { rmSync(dir, { recursive: true, force: true }); }
});

test('an exact package hit skips every Rust setup/compile step and still validates the package', () => {
  assert.doesNotMatch(action, /^\s+restore-keys:/m);
  const blocks = action.split(/^    - name: /m).slice(1);
  for (const block of blocks.filter(b => /rust-toolchain@|rust-cache@|run: cargo install|\n        crates\/zoo-crypto-wasm\/build\.sh/.test(b))) {
    assert.match(block, /if: steps\.package\.outputs\.cache-hit != 'true'/);
  }
  const verify = blocks.find(b => b.startsWith('Verify generated package'));
  assert.ok(verify.includes('zoo_crypto_wasm_bg.wasm'));
  assert.ok(!verify.includes('if:'));
});
