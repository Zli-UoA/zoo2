import { readFileSync } from 'node:fs';
import assert from 'node:assert/strict';

const yaml = readFileSync(new URL('../../.github/workflows/select-runner.yaml', import.meta.url), 'utf8');
const script = yaml.split('          script: |\n')[1].split('\n').map(line => line.replace(/^            /, '')).join('\n');
const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
const select = new AsyncFunction('core', 'context', 'github', script);
const base = ['self-hosted', 'Linux', 'X64', 'zoo2'];
const runner = (extra = [], state = {}) => ({status: 'online', busy: false, labels: [...base, ...extra].map(name => ({name})), ...state});
const cases = [
  {name: 'all capabilities', runners: [runner(['zoo2-rust', 'zoo2-docker'])], selected: ['node', 'rust', 'docker']},
  {name: 'minimal runner', runners: [runner()], selected: ['node']},
  {name: 'offline', runners: [runner([], {status: 'offline'})]},
  {name: 'busy', runners: [runner([], {busy: true})]},
  {name: 'wrong labels', runners: [{...runner(), labels: []}]},
  {name: 'API unavailable', error: true},
  {name: 'missing secret', token: false, noApi: true},
  {name: 'pull request', event: 'pull_request', noApi: true},
  {name: 'hosted override', mode: 'github', noApi: true},
  {name: 'self-hosted dispatch', mode: 'self-hosted', event: 'workflow_dispatch', noApi: true, selected: ['node', 'rust', 'docker']},
  {name: 'PR cannot force self-hosted', mode: 'self-hosted', event: 'pull_request', noApi: true},
];
for (const c of cases) {
  const outputs = {};
  let calls = 0;
  process.env.RUNNER_MODE = c.mode || 'auto';
  process.env.HAS_STATUS_TOKEN = String(c.token !== false);
  await select(
    {setOutput: (key, value) => outputs[key] = JSON.parse(value), info() {}, warning() {}},
    {eventName: c.event || 'push', repo: {owner: 'Zli-UoA', repo: 'zoo2'}},
    {rest: {actions: {listSelfHostedRunnersForRepo: {}}}, paginate: async () => {
      calls++;
      if (c.error) throw new Error('API failure');
      return c.runners || [];
    }},
  );
  for (const kind of ['node', 'rust', 'docker']) {
    const expected = c.selected?.includes(kind)
      ? [...base, ...({node: [], rust: ['zoo2-rust'], docker: ['zoo2-docker']}[kind])]
      : 'ubuntu-latest';
    assert.deepEqual(outputs[kind], expected, `${c.name}: ${kind}`);
  }
  assert.equal(calls, c.noApi ? 0 : 1, `${c.name}: API access`);
  console.log(`PASS ${c.name}`);
}
