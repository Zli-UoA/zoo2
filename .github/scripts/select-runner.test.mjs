import {readFileSync} from 'node:fs';
import assert from 'node:assert/strict';

// select-runner.yaml の script をそのまま取り出して実行する。
const yaml = readFileSync(new URL('../workflows/select-runner.yaml', import.meta.url), 'utf8');
const script = yaml.split('          script: |\n')[1].split('\n')
  .map(line => line.replace(/^ {12}/, '')).join('\n');
const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
const select = new AsyncFunction('core', 'context', 'github', script);

const runner = (name, state = {}) => ({
  name, status: 'online', busy: false, labels: [{name: 'zli-k8s'}], ...state,
});
const pull = repo => ({pull_request: {head: {repo: {full_name: repo}}}});
const cases = [
  {name: 'idle runner', runners: [runner('zli-k8s-abc-runner-1')], expected: 'zli-k8s'},
  {name: 'busy but can scale', runners: [runner('zli-k8s-a', {busy: true}), runner('zli-k8s-b', {busy: true})], expected: 'zli-k8s'},
  {name: 'all busy at max', runners: ['a', 'b', 'c', 'd'].map(n => runner(`zli-k8s-${n}`, {busy: true})), expected: 'ubuntu-latest'},
  {name: 'matched by name only', runners: [{...runner('zli-k8s-a'), labels: []}], expected: 'zli-k8s'},
  {name: 'offline', runners: [runner('zli-k8s-a', {status: 'offline'})], expected: 'ubuntu-latest'},
  {name: 'other runners only', runners: [{...runner('other-1'), labels: [{name: 'self-hosted'}]}], expected: 'ubuntu-latest'},
  {name: 'no runners', runners: [], expected: 'ubuntu-latest'},
  {name: 'API error', error: true, expected: 'ubuntu-latest'},
  {name: 'missing token', token: false, noApi: true, expected: 'ubuntu-latest'},
  {name: 'fork pull request', event: 'pull_request', payload: pull('someone/zoo2'), runners: [runner('zli-k8s-a')], noApi: true, expected: 'ubuntu-latest'},
  {name: 'same-repo pull request', event: 'pull_request', payload: pull('Zli-UoA/zoo2'), runners: [runner('zli-k8s-a')], expected: 'zli-k8s'},
  {name: 'pull_request_target from fork', event: 'pull_request_target', payload: pull('someone/zoo2'), runners: [runner('zli-k8s-a')], expected: 'zli-k8s'},
];

for (const c of cases) {
  let output;
  let calls = 0;
  process.env.HAS_STATUS_TOKEN = String(c.token !== false);
  await select(
    {setOutput: (key, value) => { assert.equal(key, 'runs-on'); output = JSON.parse(value); }, info() {}, warning() {}},
    {eventName: c.event || 'push', payload: c.payload || {}, repo: {owner: 'Zli-UoA', repo: 'zoo2'}},
    {rest: {actions: {listSelfHostedRunnersForOrg: {}}}, paginate: async (_, params) => {
      calls++;
      assert.equal(params.org, 'Zli-UoA');
      if (c.error) throw Object.assign(new Error('API failure'), {status: 403});
      return c.runners;
    }},
  );
  assert.equal(output, c.expected, c.name);
  assert.equal(calls, c.noApi ? 0 : 1, `${c.name}: API calls`);
  console.log(`PASS ${c.name}`);
}
