import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {test} from 'node:test';

const workflow = readFileSync(new URL('../workflows/cd-db-erd.yaml', import.meta.url), 'utf8');
const script = workflow.split('      - name: Check current ERD inputs before publishing\n')[1]
  .split('          script: |\n')[1].split('      - name: Deploy to Cloudflare Pages\n')[0]
  .split('\n').map(line => line.replace(/^ {12}/, '')).join('\n');
const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
const decide = new AsyncFunction('core', 'context', 'github', script);
const paths = ['infra/db/schema.sql', 'docs/erd', '.github/workflows/cd-db-erd.yaml', '.github/workflows/select-runner.yaml'];

async function run({changed, fail = false} = {}) {
  const outputs = {};
  const calls = [];
  await decide({setOutput: (key, value) => {outputs[key] = value;}, notice() {}},
    {repo: {owner: 'Zli-UoA', repo: 'zoo2'}, sha: 'built'},
    {rest: {repos: {
      getBranch: async () => ({data: {commit: {sha: 'current'}}}),
      getContent: async ({ref, path}) => {
        calls.push({ref, path});
        if (fail) throw new Error('API unavailable');
        const sha = path === changed && ref === 'current' ? 'new' : 'same';
        const entries = [{name: 'build.sh', sha}, {name: 'nested', sha: 'tree'}];
        return {data: path === 'docs/erd'
          ? (ref === 'current' ? entries.reverse() : entries)
          : {sha}};
      },
    }}});
  assert.deepEqual(new Set(calls.map(c => c.path)), new Set(paths));
  assert.ok(calls.every(c => ['built', 'current'].includes(c.ref)));
  return outputs.publish;
}

test('unrelated main commits preserve the pending ERD update', async () => {
  assert.equal(await run(), 'true');
});
for (const changed of paths) {
  test(`changed input ${changed} blocks an outdated deployment`, async () => {
    assert.equal(await run({changed}), 'false');
  });
}
test('input verification errors stop deployment', async () => {
  await assert.rejects(run({fail: true}), /API unavailable/);
});
