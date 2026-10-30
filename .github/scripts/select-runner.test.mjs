import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readFileSync, readdirSync } from 'node:fs';

const root = new URL('../../', import.meta.url);
const yaml = readFileSync(new URL('.github/workflows/select-runner.yaml', root), 'utf8');
const script = yaml.split('          script: |\n')[1].split('\n').map(l => l.replace(/^ {12}/, '')).join('\n');
const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
const select = new AsyncFunction('core', 'context', 'github', script);
const K = 'zli-kubecamp-no-docker';
const A = 'zli-k8s';
const H = 'ubuntu-latest';
const arc = (name = 'a', state = {}) => ({name: `zli-k8s-${name}`, status: 'online', busy: false, labels: [{name: A}], ...state});
const camp = (state = {}) => ({name: 'kubecamp-runner-no-docker-1', status: 'online', busy: false, labels: [{name: K}], ...state});
const busy = (n) => arc(n, {busy: true});
const pull = repo => ({pull_request: {head: {repo: {full_name: repo}}}});

const cases = [
  ['kubecamp before idle ARC', {runners: [camp(), arc()]}, K, A],
  ['kubecamp without ARC', {runners: [camp()]}, K, H],
  ['kubecamp before full ARC', {runners: [camp(), busy('a'), busy('b')]}, K, H],
  ['busy kubecamp falls back to ARC', {runners: [camp({busy: true}), arc()]}, A, A],
  ['busy kubecamp allows ARC scaling', {runners: [camp({busy: true}), busy('a')]}, A, A],
  ['busy pools fall back to hosted', {runners: [camp({busy: true}), busy('a'), busy('b')]}, H, H],
  ['offline kubecamp is ignored', {runners: [camp({status: 'offline'}), arc()]}, A, A],
  ['idle ARC', {runners: [arc()]}, A, A],
  ['busy ARC can scale', {runners: [busy('a')]}, A, A],
  ['full ARC', {runners: [busy('a'), busy('b')]}, H, H],
  ['ARC idle alongside busy', {runners: [busy('a'), arc('b')]}, A, A],
  ['offline ARC cannot scale', {runners: [arc('a', {status: 'offline'})]}, H, H],
  ['no runners', {runners: []}, H, H],
  ['unrelated runners', {runners: [arc('other', {name: 'other', labels: [{name: 'self-hosted'}]})]}, H, H],
  ['kubecamp name needs its routing label', {runners: [camp({labels: []})]}, H, H],
  ['kubecamp label matches another name', {runners: [camp({name: 'another'})]}, K, H],
  ['ARC matches scale set name', {runners: [arc('a', {labels: []})]}, A, A],
  ['ARC matches label', {runners: [arc('a', {name: 'another'})]}, A, A],
  ['missing labels are safe', {runners: [camp({labels: undefined})]}, H, H],
  ['API error keeps first eligible pools', {error: true}, K, A],
  ['missing token keeps first eligible pools', {token: false}, K, A],
  ['fork PR is hosted', {event: 'pull_request', payload: pull('someone/repo')}, H, H],
  ['same-repo PR prefers kubecamp', {event: 'pull_request', payload: pull('Zli-UoA/repo'), runners: [camp(), arc()]}, K, A],
  ['trusted pull_request_target can use kubecamp', {event: 'pull_request_target', payload: pull('someone/repo'), runners: [camp(), arc()]}, K, A],
];
for (const [name, input, expected, dockerExpected] of cases) {
  test(`select-runner: ${name}`, async () => {
    const outputs = {};
    let calls = 0;
    process.env.HAS_STATUS_TOKEN = String(input.token !== false);
    await select({setOutput(key, value) {outputs[key] = JSON.parse(value);}, info() {}, warning() {}},
      {eventName: input.event || 'push', payload: input.payload || {}, repo: {owner: 'Zli-UoA', repo: 'repo'}},
      {rest: {actions: {listSelfHostedRunnersForOrg: {}}}, paginate: async (_, params) => {
        calls++;
        assert.equal(params.org, 'Zli-UoA');
        assert.equal(params.per_page, 100);
        assert.equal(params.request.timeout, 10000);
        if (input.error) throw Object.assign(new Error('private response must not be logged'), {status: 403});
        return input.runners;
      }});
    assert.deepEqual(outputs, {'runs-on': expected, 'docker-runs-on': dockerExpected});
    assert.equal(calls, input.token === false || input.event === 'pull_request' && expected === H ? 0 : 1);
  });
}

const dockerJobs = {"ci-backend.yaml": ["deploy-image", "e2e"], "ci-frontend.yaml": ["container", "proxy"], "ci-frontend-auth-e2e.yaml": ["oidc-e2e"], "cd-images.yaml": ["build"]};
test('every workload uses the right output and protects fork PRs', () => {
  const dir = new URL('.github/workflows/', root);
  let checked = 0;
  for (const file of readdirSync(dir).filter(x => /\.ya?ml$/.test(x) && !x.startsWith('select-runner'))) {
    const text = readFileSync(new URL(file, dir), 'utf8');
    const jobs = text.split(/^jobs:\s*$/m)[1];
    const matches = [...jobs.matchAll(/^  ([\w-]+):\s*$/gm)];
    for (let i = 0; i < matches.length; i++) {
      const id = matches[i][1];
      const block = jobs.slice(matches[i].index, matches[i + 1]?.index);
      if (id === 'runner') {
        assert.ok(!block.includes('if: vars.CI_RUNNER'), `${file}: manual variable must not skip selection`);
        continue;
      }
      if (file === 'container.yml') {
        assert.match(block, /fromJSON\(inputs\.runs-on\)/);
        continue;
      }
      const docker = (dockerJobs[file] || []).includes(id);
      assert.ok(block.includes(`needs.runner.outputs.${docker ? 'docker-runs-on' : 'runs-on'}`), `${file}/${id}: wrong pool`);
      const line = block.split('\n').find(x => x.startsWith('    runs-on:'));
      if (line) {
        assert.ok(line.includes(`github.event.pull_request.head.repo.fork && '${H}' ||`), `${file}/${id}: fork guard`);
      }
      if (/docker\/(?:setup-buildx|build-push)-action|docker (?:run|compose|build)\b|scripts\/test-(?:alertmanager|log-sanitization)/.test(block)) {
        assert.ok(docker, `${file}/${id}: Docker daemon use routed to kubecamp`);
      }
      checked++;
    }
  }
  assert.ok(checked > 0);
});
test('selector host overrides do not override workload priorities', () => {
  assert.match(yaml, /vars\.CI_SELECTOR_RUNNER \|\| vars\.CI_RUNNER/);
  assert.equal(Number(script.match(/const MAX_RUNNERS = (\d+);/)[1]), 2);
});
