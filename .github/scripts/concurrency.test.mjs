import assert from 'node:assert/strict';
import {readdirSync, readFileSync} from 'node:fs';
import test from 'node:test';

const dir = new URL('../workflows/', import.meta.url);
// デプロイ・リリース系は止めると中途半端になるので、queue や cancel-in-progress: false を使う。ここでは CI だけを見る。
const workflows = readdirSync(dir).filter(x => /^ci-.*\.ya?ml$/.test(x)).map(file => {
  const text = readFileSync(new URL(file, dir), 'utf8');
  const triggers = text.split(/^jobs:\s*$/m)[0];
  return {file, text, pushes: /^  push:/m.test(triggers), concurrency: text.match(/^concurrency:\n((?:  .*\n)+)/m)?.[1]};
});

// main の実行を cancel すると、成功したジョブの最後に保存するキャッシュ（rust-cache など）が main に残らず、
// 後続の PR が cold build になる。main では実行中のものを止めず、PR と merge queue だけ止める。
test('workflows that run on push to main do not cancel in-progress main runs', () => {
  const checked = workflows.filter(w => w.pushes && w.concurrency);
  assert.ok(checked.length > 0);
  for (const {file, concurrency} of checked) {
    const cancel = concurrency.match(/^  cancel-in-progress: (.*)$/m)?.[1];
    assert.ok(cancel, `${file}: cancel-in-progress is required`);
    if (cancel === 'false') continue;
    assert.equal(cancel, "${{ github.ref != 'refs/heads/main' }}", `${file}: cancel-in-progress must exclude main`);
  }
});

test('push CI workflows group runs per PR or ref so that PRs never cancel each other', () => {
  for (const {file, concurrency} of workflows.filter(w => w.pushes && w.concurrency)) {
    assert.match(concurrency, /group: .*(github\.event\.pull_request\.number \|\| github\.ref|github\.ref)/, `${file}: group`);
  }
});
