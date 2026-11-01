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
const pull = repo => ({pull_request: {head: {repo: {full_name: repo}}}});
const PRICES = {'Actions Linux': 0.006, 'Actions Windows': 0.01, 'Actions storage': 0.00033602};
const usage = (repositoryName, quantity, sku = 'Actions Linux') => ({
  product: 'actions', sku, repositoryName, quantity, grossAmount: quantity * PRICES[sku],
  unitType: sku === 'Actions storage' ? 'GigabyteHours' : 'Minutes',
});
const privateRepo = {repository: {private: true}};
const privateRepos = [{name: 'AuthZ'}, {name: 'repo'}];

const cases = [
  ['kubecamp first', {runners: [camp(), arc()]}, K, H],
  ['busy kubecamp falls back to hosted before ARC', {runners: [camp({busy: true}), arc()]}, H, H],
  ['offline kubecamp is ignored', {runners: [camp({status: 'offline'}), arc()]}, H, H],
  ['no runners', {runners: []}, H, H],
  ['kubecamp name needs its routing label', {runners: [camp({labels: []})]}, H, H],
  ['kubecamp label matches another name', {runners: [camp({name: 'another'})]}, K, H],
  ['missing labels are safe', {runners: [camp({labels: undefined})]}, H, H],
  ['API error keeps kubecamp', {error: true}, K, H],
  ['missing token keeps kubecamp', {token: false}, K, H],
  ['fork PR is hosted', {event: 'pull_request', payload: pull('someone/repo')}, H, H],
  ['same-repo PR prefers kubecamp', {event: 'pull_request', payload: {...pull('Zli-UoA/repo'), repository: {private: false}}, runners: [camp()]}, K, H],
  ['trusted pull_request_target can use kubecamp', {event: 'pull_request_target', payload: {...pull('someone/repo'), repository: {private: false}}, runners: [camp()]}, K, H],
  ['private within included minutes uses hosted', {payload: privateRepo, usage: [usage('AuthZ', 1000), usage('repo', 899)]}, H, H],
  ['private out of minutes uses ARC', {payload: privateRepo, usage: [usage('AuthZ', 1000), usage('repo', 900)]}, A, A],
  ['private out of minutes still prefers kubecamp', {payload: privateRepo, runners: [camp()], usage: [usage('AuthZ', 1900)]}, K, A],
  ['other OS usage counts by price', {payload: privateRepo, usage: [usage('repo', 1500, 'Actions Windows')]}, A, A],
  ['public repositories and storage are not counted', {payload: privateRepo, usage: [usage('zoo2', 9000), usage('repo', 9000, 'Actions storage'), usage('repo', 10)]}, H, H],
  ['documented billing format is counted', {payload: {repository: {private: true}}, usage: [{...usage('Zli-UoA/Repo', 1900), product: 'Actions'}]}, A, A],
  ['unknown visibility is checked as private', {payload: {}, usage: [usage('repo', 1900)]}, A, A],
  ['billing access error names the call and permissions', {payload: privateRepo, billingError: true,
    warning: 'Hosted minutes unavailable: GET /organizations/{org}/settings/billing/usage (403; accepted permissions: administration=read)'}, A, A],
  ['repository list error names the call', {payload: privateRepo, reposError: true, warning: 'Hosted minutes unavailable: GET /orgs/{org}/repos (403)'}, A, A],
  ['private without token uses ARC', {payload: privateRepo, token: false}, K, A],
];
for (const [name, input, expected, hostedExpected] of cases) {
  test(`select-runner: ${name}`, async () => {
    const outputs = {};
    const calls = {runners: 0, repos: 0, billing: 0};
    const warnings = [];
    const payload = input.payload || {repository: {private: false}};
    const fork = input.event === 'pull_request' && payload.pull_request.head.repo.full_name !== 'Zli-UoA/repo';
    process.env.HAS_STATUS_TOKEN = String(input.token !== false);
    const rest = {actions: {listSelfHostedRunnersForOrg: {}}, repos: {listForOrg: {}}};
    await select({setOutput(key, value) {outputs[key] = JSON.parse(value);}, info() {}, warning(m) {warnings.push(m);}},
      {eventName: input.event || 'push', payload, repo: {owner: 'Zli-UoA', repo: 'repo'}},
      {rest, paginate: async (method, params) => {
        assert.equal(params.org, 'Zli-UoA');
        assert.equal(params.per_page, 100);
        assert.equal(params.request.timeout, 10000);
        if (method === rest.repos.listForOrg) {
          calls.repos++;
          if (input.reposError) throw Object.assign(new Error('private response must not be logged'), {status: 403});
          assert.equal(params.type, 'private');
          return privateRepos;
        }
        calls.runners++;
        if (input.error) throw Object.assign(new Error('private response must not be logged'), {status: 403});
        return input.runners || [];
      }, request: async (route, params) => {
        calls.billing++;
        assert.equal(route, 'GET /organizations/{org}/settings/billing/usage');
        assert.equal(params.org, 'Zli-UoA');
        assert.ok(params.year >= 2026 && params.month >= 1 && params.month <= 12);
        if (input.billingError) throw Object.assign(new Error('private response must not be logged'), {status: 403, response: {headers: {'x-accepted-github-permissions': 'administration=read'}}});
        return {data: {usageItems: input.usage || []}};
      }});
    assert.deepEqual(outputs, {'runs-on': expected, 'docker-runs-on': hostedExpected, 'large-runs-on': hostedExpected});
    const checksMinutes = !fork && input.token !== false && payload.repository?.private !== false;
    assert.deepEqual(calls, {
      runners: fork || input.token === false ? 0 : 1,
      repos: checksMinutes ? 1 : 0,
      billing: checksMinutes && !input.reposError ? 1 : 0,
    });
    assert.ok(warnings.every(m => !m.includes('private response')));
    if (input.warning) assert.deepEqual(warnings, [input.warning]);
  });
}

const fixedJobs = {"flux-status.yaml": ["report"], "cd-preview-label.yaml": ["label"], "cd-image-cleanup.yaml": ["cleanup"], "ci-flux-status.yaml": ["test"], "cd-images.yaml": ["plan", "promote", "publish"], "cd-storybook.yaml": ["deploy"], "terraform-r2.yaml": ["validate", "plan"], "ci-frontend.yaml": ["check", "unit-test"], "automerge-gate.yaml": ["gate"]};
const dockerJobs = {"ci-compose.yaml": ["container"], "ci-github-bot.yaml": ["test", "publish"], "ci-backend.yaml": ["deploy-image", "e2e"], "ci-frontend.yaml": ["container", "proxy"], "ci-frontend-auth-e2e.yaml": ["oidc-e2e"], "cd-images.yaml": ["build"]};
// Kotlin builds use hosted runners for memory and the preinstalled Android SDK.
const hostedJobs = {"ci-compose.yaml": ["web"], "ci-android.yaml": ["android", "publish-main"], "cd-storybook.yaml": ["build"],
  // merge_group には fork の情報がなく、キューに入った PR のコードを self-hosted で動かさない。
  "merge-queue.yaml": ["changes", "all-passed"]};
// macOS 向けのビルドがあるため、GitHub ホストの実行先だけを使うジョブ。PR では動かないワークフローに限る。
const releaseJobs = {"cd-tui-release.yaml": ["version", "build", "release"]};
const queued = new Set(['merge-queue.yaml', ...[...readFileSync(new URL('.github/workflows/merge-queue.yaml', root), 'utf8')
  .matchAll(/^    uses: \.\/\.github\/workflows\/([\w-]+\.yaml)$/gm)].map(m => m[1])]);
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
      // reusable workflow の呼び出しは実行先を持たず、呼び出し先の workflow で検証する。
      if (id !== 'runner' && /^    uses: \.\/\.github\/workflows\//m.test(block)) continue;
      if (id === 'runner') {
        assert.ok(!block.includes('if: vars.CI_RUNNER'), `${file}: manual variable must not skip selection`);
        continue;
      }
      if (file === 'container.yml') {
        assert.match(block, /fromJSON\(inputs\.runs-on\)/);
        continue;
      }
      if ((releaseJobs[file] || []).includes(id)) {
        assert.doesNotMatch(text.split(/^jobs:\s*$/m)[0], /pull_request/, `${file}: hosted-only workflows must not run on PRs`);
        assert.match(block, /^    runs-on: (?:ubuntu-latest|\$\{\{ matrix\.os \}\})$/m, `${file}/${id}: hosted pool`);
        assert.doesNotMatch(block, /needs\.runner\./, `${file}/${id}: selector dependency`);
        checked++;
        continue;
      }
      const docker = (dockerJobs[file] || []).includes(id);
      const large = file === 'ci-frontend.yaml' && id === 'browser-test';
      const hosted = (hostedJobs[file] || []).includes(id);
      const fixed = (fixedJobs[file] || []).includes(id);
      if (hosted) {
        assert.match(block, /^    runs-on: ubuntu-latest$/m, `${file}/${id}: hosted for all PRs`);
        assert.doesNotMatch(block, /^    needs:.*\brunner\b|needs\.runner\./m, `${file}/${id}: selector dependency`);
      } else if (fixed) {
        assert.match(block, /runs-on:.*\|\| 'zli-kubecamp-no-docker' \}\}/, `${file}/${id}: fixed pool`);
        assert.doesNotMatch(block, /^    needs:.*\brunner\b|needs\.runner\./m, `${file}/${id}: selector dependency`);
      } else {
        assert.ok(block.includes(`needs.runner.outputs.${large ? 'large-runs-on' : docker ? 'docker-runs-on' : 'runs-on'}`), `${file}/${id}: wrong pool`);
        if (docker) {
          assert.ok(block.includes(`docker-runs-on || '"${H}"'`), `${file}/${id}: Docker fallback must be hosted`);
        }
      }
      const line = block.split('\n').find(x => x.startsWith('    runs-on:'));
      if (line && !hosted) {
        // merge_group には元 PR の fork 情報がないため、キューで動く workflow は常に GitHub-hosted にする。
        const fork = queued.has(file) ? "(github.event.pull_request.head.repo.fork || github.event_name == 'merge_group')" : 'github.event.pull_request.head.repo.fork';
        assert.ok(line.includes(`${fork} && '${H}' ||`), `${file}/${id}: fork guard`);
      }
      if (/docker\/(?:setup-buildx|build-push)-action|docker (?:run|compose|build)\b|scripts\/test-(?:alertmanager|log-sanitization)/.test(block)) {
        assert.ok(docker, `${file}/${id}: Docker daemon use routed to kubecamp`);
      }
      checked++;
    }
  }
  assert.ok(checked > 0);
});
test('Kotlin build workflows keep hosted builds independent of runner selection', () => {
  const storybook = readFileSync(new URL('.github/workflows/cd-storybook.yaml', root), 'utf8');
  assert.doesNotMatch(storybook, /^  runner:/m);
  for (const [file, ids] of Object.entries(hostedJobs)) {
    const text = readFileSync(new URL(`.github/workflows/${file}`, root), 'utf8');
    for (const id of ids) {
      const block = text.split(`  ${id}:\n`)[1].split(/^  [\w-]+:\s*$/m)[0];
      assert.match(block, /^    runs-on: ubuntu-latest$/m);
      assert.doesNotMatch(block, /needs\.runner|^    needs: runner$/m);
    }
  }
});
test('selector host overrides do not override workload priorities', () => {
  assert.match(yaml, /vars\.CI_SELECTOR_RUNNER \|\| vars\.CI_RUNNER/);
  assert.doesNotMatch(script, /arc\.length\s*</);
});

test('large jobs use the ARC/hosted output when kubecamp memory is insufficient', () => {
  assert.match(yaml, /large-runs-on:\n\s+description: [^\n]+\n\s+value: \$\{\{ jobs.select.outputs.large-runs-on \}\}/);
  assert.match(yaml, /large-runs-on: \$\{\{ steps.select.outputs.large-runs-on \}\}/);
});

test('fixed-only workflows do not create a selector job', () => {
  for (const [file, ids] of Object.entries(fixedJobs)) {
    const text = readFileSync(new URL(`.github/workflows/${file}`, root), 'utf8');
    const jobs = text.split(/^jobs:\s*$/m)[1];
    const names = [...jobs.matchAll(/^  ([\w-]+):\s*$/gm)].map(m => m[1]);
    if (["cd-images.yaml", "cd-storybook.yaml", "ci-frontend.yaml"].includes(file)) continue;
    assert.deepEqual(names.sort(), [...ids].sort(), `${file}: only fixed jobs are expected`);
    assert.doesNotMatch(jobs, /^  runner:/m);
  }
});

test('fixed jobs keep their result dependencies and trusted deployment boundary', () => {
  const storybook = readFileSync(new URL('.github/workflows/cd-storybook.yaml', root), 'utf8').split('  deploy:')[1];
  assert.match(storybook, /needs: \[build\]/);
  assert.doesNotMatch(storybook, /actions\/checkout/);
  for (const file of ['cd-image-cleanup.yaml', 'cd-preview-label.yaml']) {
    const text = readFileSync(new URL(`.github/workflows/${file}`, root), 'utf8');
    assert.match(text, /ref: \$\{\{ github\.event\.repository\.default_branch \}\}/);
  }
});
