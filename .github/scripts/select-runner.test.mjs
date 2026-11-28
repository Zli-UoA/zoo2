import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readFileSync, readdirSync } from 'node:fs';
import { selectWorkflows } from './merge-queue-changes.mjs';

const root = new URL('../../', import.meta.url);
const yaml = readFileSync(new URL('.github/workflows/select-runner.yaml', root), 'utf8');
const script = yaml.split('          script: |\n')[1].split('\n').map(l => l.replace(/^ {12}/, '')).join('\n');
const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
// 固定した actions/github-script（ed597411）が、スクリプトに渡す引数と同じもの。これ以外（getOctokit など）は渡さない。
// 本番にない関数をテストが供給すると、本番で ReferenceError になるのに、テストが通ってしまう。
const SCRIPT_ARGS = ['context', 'core', 'github', 'octokit', 'exec', 'glob', 'io', 'require', '__original_require__'];
const compiled = new AsyncFunction(...SCRIPT_ARGS, script);
const select = (core, context, github) => compiled(context, core, github, github, {}, {}, {}, () => { throw new Error('require is not available'); }, () => { throw new Error('require is not available'); });
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
  ['public bypasses self-hosted and billing APIs', {payload: {repository: {private: false}}, runners: [camp()], error: true, billingError: true}, H, H],
  ['public without status token is hosted', {payload: {repository: {private: false}}, token: false}, H, H],
  ['merge group is hosted even with exhausted minutes', {event: 'merge_group', payload: privateRepo, usage: [usage('repo', 2000)]}, H, H],
  ['kubecamp first', {runners: [camp(), arc()]}, K, H],
  ['busy kubecamp falls back to hosted before ARC', {runners: [camp({busy: true}), arc()]}, H, H],
  ['offline kubecamp is ignored', {runners: [camp({status: 'offline'}), arc()]}, H, H],
  ['no runners', {runners: []}, H, H],
  ['kubecamp name needs its routing label', {runners: [camp({labels: []})]}, H, H],
  ['kubecamp label matches another name', {runners: [camp({name: 'another'})]}, K, H],
  ['missing labels are safe', {runners: [camp({labels: undefined})]}, H, H],
  ['API error keeps kubecamp', {error: true}, K, H],
  ['private missing token keeps kubecamp and ARC', {token: false}, K, A],
  ['fork PR is hosted', {event: 'pull_request', payload: pull('someone/repo')}, H, H],
  ['same-repo PR prefers kubecamp', {event: 'pull_request', payload: {...pull('Zli-UoA/repo'), repository: {private: true}}, runners: [camp()]}, K, H],
  ['trusted pull_request_target can use kubecamp', {event: 'pull_request_target', payload: {...pull('someone/repo'), repository: {private: true}}, runners: [camp()]}, K, H],
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
  // 重いジョブだけ、public でも CI_LARGE_RUNNER で heavy に切り替えられる。
  ['public with CI_LARGE_RUNNER sends only large jobs to heavy', {payload: {repository: {private: false}}, large: 'zli-k8s-heavy', largeExpected: 'zli-k8s-heavy', heavyCalls: {runs: 6, jobs: 0}}, H, H],
  ['public with CI_LARGE_RUNNER does not need the status token', {payload: {repository: {private: false}}, token: false, large: 'zli-k8s-heavy', largeExpected: 'zli-k8s-heavy', heavyCalls: {runs: 6, jobs: 0}}, H, H],
  ['public with another CI_LARGE_RUNNER stays hosted', {payload: {repository: {private: false}}, large: 'zli-k8s', warning: 'CI_LARGE_RUNNER must be empty or zli-k8s-heavy; using hosted'}, H, H],
  ['fork PR ignores CI_LARGE_RUNNER', {event: 'pull_request', payload: {...pull('someone/repo'), repository: {private: false}}, large: 'zli-k8s-heavy'}, H, H],
  ['merge group ignores CI_LARGE_RUNNER', {event: 'merge_group', payload: {repository: {private: false}}, large: 'zli-k8s-heavy'}, H, H],
  // heavy が埋まっているときは、重いジョブだけ Hosted に逃がす（選択時点の使用状況。予約ではない）。
  ...(() => {
    const pub = {repository: {private: false}};
    const base = {payload: pub, large: 'zli-k8s-heavy'};
    const job = (status, label = 'zli-k8s-heavy') => ({status, labels: [label]});
    return [
      ['heavy idle: no heavy jobs', {...base, largeExpected: 'zli-k8s-heavy', heavyCalls: {runs: 6, jobs: 0}}, H, H],
      ['heavy has a free runner: one running job', {...base, largeExpected: 'zli-k8s-heavy', heavyRuns: {'ci-backend.yaml:in_progress': [{id: 1}]}, heavyJobs: {1: [job('in_progress')]}, heavyCalls: {runs: 6, jobs: 1}}, H, H],
      ['central PR CI consumes heavy capacity', {...base, largeExpected: H, heavyRuns: {'pr-ci.yaml:in_progress': [{id: 1}]}, heavyJobs: {1: [job('in_progress'), job('queued')]}, heavyCalls: {runs: 6, jobs: 1}}, H, H],
      ['heavy full: two running jobs go to hosted', {...base, largeExpected: H, heavyRuns: {'ci-backend.yaml:in_progress': [{id: 1}]}, heavyJobs: {1: [job('in_progress'), job('in_progress')]}, heavyCalls: {runs: 2, jobs: 1}}, H, H],
      ['heavy full: running and queued jobs across workflows', {...base, largeExpected: H, heavyRuns: {'ci-backend.yaml:in_progress': [{id: 1}], 'ci-frontend.yaml:queued': [{id: 2}]}, heavyJobs: {1: [job('in_progress')], 2: [job('queued')]}, heavyCalls: {runs: 4, jobs: 4}}, H, H],
      ['a full queued run is counted when the second listing fails', {...base, largeExpected: H, heavyError: 2, heavyRuns: {'ci-backend.yaml:queued': [{id: 1}]}, heavyJobs: {1: [job('queued'), job('queued')]}, heavyCalls: {runs: 2, jobs: 1}}, H, H],
      ['an incomplete listing warns when the queued run is below capacity', {...base, largeExpected: 'zli-k8s-heavy', heavyError: 2, heavyRuns: {'ci-backend.yaml:queued': [{id: 1}]}, heavyJobs: {1: [job('queued')]}, warning: 'Heavy runner usage unavailable (403); using zli-k8s-heavy', heavyCalls: {runs: 2, jobs: 1}}, H, H],
      ['the latest run can finish during confirmation', {...base, largeExpected: 'zli-k8s-heavy', heavyRuns: {'ci-backend.yaml:in_progress': [{id: 1}], 'ci-frontend.yaml:queued': [{id: 2}]}, heavyJobs: {1: [job('in_progress')], 2: [job('queued')]}, finishedRun: '2', heavyCalls: {runs: 6, jobs: 4}}, H, H],
      ['a completed job from an earlier run does not make heavy full', {...base, largeExpected: 'zli-k8s-heavy', heavyRuns: {'ci-backend.yaml:in_progress': [{id: 1}], 'ci-frontend.yaml:queued': [{id: 2}]}, heavyJobs: {1: [...Array.from({length: 100}, () => job('completed')), job('in_progress')], 2: [job('queued')]}, finishedRun: '1', heavyCalls: {runs: 6, jobs: 4}}, H, H],
      // 後続ページの取得が失敗しても、取得済みのページの run を確認する。2件が分かれば Hosted、分からなければ警告して heavy のまま。
      ['a failing second page still checks the runs already fetched: full means hosted', {...base, largeExpected: H, heavyError: 2, heavyRuns: {'ci-backend.yaml:queued': [{id: 1}, ...Array.from({length: 99}, (_, i) => ({id: 100 + i}))]}, heavyJobs: {1: [job('queued'), job('queued')]}, heavyCalls: {runs: 2, jobs: 1}}, H, H],
      ['a failing second page with many fetched runs runs out of budget and keeps heavy', {...base, largeExpected: 'zli-k8s-heavy', heavyError: 2, heavyRuns: {'ci-backend.yaml:queued': [{id: 1}, ...Array.from({length: 99}, (_, i) => ({id: 100 + i}))]}, heavyJobs: {1: [job('queued')]}, warning: 'Heavy runner usage unknown: the API request budget (30) ran out; using zli-k8s-heavy', heavyCalls: {runs: 2, jobs: 28}}, H, H],
      ['a failing second page warns and keeps heavy when the fetched runs are not full', {...base, largeExpected: 'zli-k8s-heavy', heavyError: 2, heavyRuns: {'ci-backend.yaml:queued': [{id: 1}]}, heavyJobs: {1: [job('queued')]}, warning: 'Heavy runner usage unavailable (403); using zli-k8s-heavy', heavyCalls: {runs: 2, jobs: 1}}, H, H],
      // 先に数えた run の再取得だけで2件に達したら、現在の run の再取得が失敗しても、Hosted を選ぶ。
      ['the refreshed earlier runs alone fill heavy even if the current run cannot be refreshed', {...base, largeExpected: H, heavyRuns: {'ci-backend.yaml:queued': [{id: 1}, {id: 2}]}, jobSequence: {1: [[job('queued')], [job('queued'), job('queued')]], 2: [[job('queued')], 'error']}, heavyCalls: {runs: 2, jobs: 3}}, H, H],
      ['a run moving out of queued is counted', {...base, largeExpected: H, movingRun: true, heavyJobs: {1: [job('in_progress'), job('in_progress')]}, heavyCalls: {runs: 2, jobs: 1}}, H, H],
      ['a padded runner label is rejected', {...base, large: 'zli-k8s-heavy ', largeExpected: H, warning: 'CI_LARGE_RUNNER must be empty or zli-k8s-heavy; using hosted'}, H, H],
      ['other labels and finished jobs are not counted', {...base, largeExpected: 'zli-k8s-heavy', heavyRuns: {'ci-backend.yaml:in_progress': [{id: 1}]}, heavyJobs: {1: [job('in_progress', 'ubuntu-latest'), job('completed'), job('waiting'), job('queued', 'zli-k8s')]}, heavyCalls: {runs: 6, jobs: 1}}, H, H],
      ['the current run is not counted', {...base, runId: 1, largeExpected: 'zli-k8s-heavy', heavyRuns: {'ci-backend.yaml:in_progress': [{id: 1}]}, heavyJobs: {1: [job('in_progress'), job('in_progress')]}, heavyCalls: {runs: 6, jobs: 0}}, H, H],
      ['unreadable heavy usage keeps heavy', {...base, largeExpected: 'zli-k8s-heavy', heavyError: true, warning: 'Heavy runner usage unavailable (403); using zli-k8s-heavy', heavyCalls: {runs: 1, jobs: 0}}, H, H],
      ['a run listed as in_progress and queued is counted once', {...base, largeExpected: 'zli-k8s-heavy', heavyRuns: {'ci-backend.yaml:in_progress': [{id: 1}], 'ci-backend.yaml:queued': [{id: 1}]}, heavyJobs: {1: [job('in_progress')]}, heavyCalls: {runs: 6, jobs: 1}}, H, H],
      ['callers that do not use large-runs-on do not read heavy usage', {...base, largeCaller: false, largeExpected: H}, H, H],
      ['slow heavy usage times out and keeps heavy', {...base, largeExpected: 'zli-k8s-heavy', heavyHang: 200, heavyTimeout: true, warning: 'Heavy runner usage unavailable (timeout); using zli-k8s-heavy', heavyCalls: {runs: 1, jobs: 0}}, H, H],
      // リクエストの総数（ページを含む）が予算（30）を超えると、heavy の使用状況は不明として、heavy のままにする。
      ['API requests are bounded and an exhausted budget is reported', {...base, largeExpected: 'zli-k8s-heavy', warning: 'Heavy runner usage unknown: the API request budget (30) ran out; using zli-k8s-heavy', heavyRuns: {'ci-backend.yaml:in_progress': Array.from({length: 50}, (_, i) => ({id: 100 + i}))}, heavyCalls: {runs: 2, jobs: 28}}, H, H],
      ['extra pages count against the same budget', {...base, largeExpected: 'zli-k8s-heavy', warning: 'Heavy runner usage unknown: the API request budget (30) ran out; using zli-k8s-heavy', heavyRuns: {'ci-backend.yaml:in_progress': Array.from({length: 250}, (_, i) => ({id: 100 + i}))}, heavyCalls: {runs: 4, jobs: 26}}, H, H],
      ['paging ends at the first short page', {...base, largeExpected: 'zli-k8s-heavy', heavyRuns: {'ci-backend.yaml:in_progress': Array.from({length: 100}, (_, i) => ({id: 100 + i}))}, warning: 'Heavy runner usage unknown: the API request budget (30) ran out; using zli-k8s-heavy', heavyCalls: {runs: 3, jobs: 27}}, H, H],
    ];
  })(),
  ['private ignores CI_LARGE_RUNNER', {payload: privateRepo, large: 'zli-k8s-heavy', runners: [camp()], usage: [usage('repo', 1000)]}, K, H],
];
for (const [name, input, expected, hostedExpected] of cases) {
  test(`select-runner: ${name}`, async () => {
    const outputs = {};
    const calls = {runners: 0, repos: 0, billing: 0};
    const warnings = [];
    const payload = input.payload || privateRepo;
    const fork = input.event === 'pull_request' && payload.pull_request.head.repo.full_name !== 'Zli-UoA/repo';
    process.env.HAS_STATUS_TOKEN = String(input.token !== false);
    process.env.LARGE_RUNNER = input.large || '';
    process.env.LARGE_CALLER = String(input.largeCaller !== false);
    process.env.HEAVY_TIMEOUT_MS = input.heavyTimeout ? '50' : '';
    process.env.ACTIONS_READ_TOKEN = 'workflow-token';
    const rest = {actions: {listSelfHostedRunnersForOrg: {}}, repos: {listForOrg: {}}};
    const heavy = {runs: 0, jobs: 0, tokens: new Set(), aborted: undefined};
    // heavy の使用状況は、ワークフロー自身のトークンで、Node の fetch から、公開リポジトリの Actions のジョブ一覧を直接読む。
    // Runner の一覧は読まない。状態確認用のトークン（PAT）のクライアント（github）は使わない。
    const jobReads = new Map();
    const originalFetch = globalThis.fetch;
    globalThis.fetch = async (url, init) => {
      const u = new URL(url);
      const [, , owner, repo, , kind, ...rest2] = u.pathname.split('/');
      assert.equal(u.origin, 'https://api.github.com');
      assert.equal(`${owner}/${repo}`, 'Zli-UoA/repo');
      assert.equal(u.searchParams.get('per_page'), '100');
      heavy.tokens.add(init.headers.authorization);
      assert.equal(init.headers['x-github-api-version'], '2022-11-28');
      const hang = async () => {
        if (!input.heavyHang) return;
        // 期限（abort）が来たら、実行中のリクエストが止まること。
        await new Promise((resolve, reject) => {
          const timer = setTimeout(resolve, input.heavyHang);
          init.signal.addEventListener('abort', () => { heavy.aborted = true; clearTimeout(timer); reject(Object.assign(new Error('aborted'), {name: 'AbortError'})); });
        });
      };
      const paged = (items, key) => {
        const n = Number(u.searchParams.get('page'));
        return {ok: true, status: 200, json: async () => ({[key]: items.slice((n - 1) * 100, n * 100)})};
      };
      if (kind === 'workflows') {
        heavy.runs++;
        if (input.heavyError === true || input.heavyError === heavy.runs) return {ok: false, status: 403, json: async () => { throw new Error('response must not be read'); }};
        await hang();
        const [workflow] = rest2;
        if (input.movingRun && workflow === 'ci-backend.yaml') {
          const activeStatus = heavy.runs === 1 ? 'queued' : 'in_progress';
          return paged(u.searchParams.get('status') === activeStatus ? [{id: 1}] : [], 'workflow_runs');
        }
        return paged((input.heavyRuns || {})[`${workflow}:${u.searchParams.get('status')}`] || [], 'workflow_runs');
      }
      assert.equal(kind, 'runs');
      heavy.jobs++;
      assert.equal(u.searchParams.get('filter'), 'latest');
      const [runId] = rest2;
      if (input.movingRun && heavy.runs < 2) return paged([], 'jobs');
      if (u.searchParams.get('page') === '1') {
        jobReads.set(runId, (jobReads.get(runId) || 0) + 1);
      }
      // 同じ run のジョブを読むたびに、順番に違う結果を返す（'error' は HTTP 503）。
      const sequence = (input.jobSequence || {})[runId];
      if (sequence) {
        const entry = sequence[Math.min(jobReads.get(runId), sequence.length) - 1];
        if (entry === 'error') return {ok: false, status: 503, json: async () => { throw new Error('response must not be read'); }};
        return paged(entry, 'jobs');
      }
      if (input.finishedRun === runId && jobReads.get(runId) > 1) return paged([], 'jobs');
      return paged((input.heavyJobs || {})[runId] || [], 'jobs');
    };
    try {
      await select({setOutput(key, value) {outputs[key] = JSON.parse(value);}, info() {}, warning(m) {warnings.push(m);}},
        {eventName: input.event || 'push', payload, repo: {owner: 'Zli-UoA', repo: 'repo'}, runId: input.runId},
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
      assert.deepEqual(outputs, {'runs-on': expected, 'docker-runs-on': hostedExpected, 'large-runs-on': input.largeExpected ?? hostedExpected});
      const hostedOnly = fork || input.event === 'merge_group' || payload.repository?.private === false;
      const checksMinutes = !hostedOnly && input.token !== false;
      assert.deepEqual(calls, {
        runners: hostedOnly || input.token === false ? 0 : 1,
        repos: checksMinutes ? 1 : 0,
        billing: checksMinutes && !input.reposError ? 1 : 0,
      });
      // 期限が来たあとに、リクエストが続かないこと（遅い応答が返ったあとも、次のリクエストを出さない）。
      if (input.heavyHang) await new Promise(resolve => setTimeout(resolve, input.heavyHang + 100));
      const {tokens, aborted, ...calledHeavy} = heavy;
      if (input.heavyCalls) {
        assert.deepEqual(calledHeavy, input.heavyCalls);
        assert.deepEqual([...tokens], ['Bearer workflow-token'], 'Actions reads use the workflow token, not the status token');
      } else {
        assert.deepEqual(calledHeavy, {runs: 0, jobs: 0}, 'heavy usage is read only when CI_LARGE_RUNNER selects heavy');
        assert.equal(tokens.size, 0);
      }
      if (input.heavyHang) assert.equal(aborted, true, 'the deadline aborts the request in flight');
      assert.ok(warnings.every(m => !m.includes('private response') && !m.includes('response must not be logged')));
      if (input.warning) assert.deepEqual(warnings, [input.warning]);
      else if (input.heavyCalls) assert.deepEqual(warnings, []);
    } finally {
      globalThis.fetch = originalFetch;
    }
  });
}

const fixedJobs = {"pr-assign.yaml": ["assign"], "flux-status.yaml": ["report"],"cd-preview-label.yaml": ["label"], "cd-image-cleanup.yaml": ["cleanup"], "ci-flux-status.yaml": ["test"], "cd-images.yaml": ["promote", "publish"], "cd-storybook.yaml": ["deploy"], "cd-db-erd.yaml": ["deploy"], "deployment-badges.yaml": ["test", "publish"], "terraform-r2.yaml": ["apply", "plan", "validate"]};
const dockerJobs = {"cd-db-erd.yaml": ["build"], "ci-github-bot.yaml": ["test", "publish"], "ci-backend.yaml": ["deploy-image", "e2e", "shared-emoji-projection"], "ci-frontend.yaml": ["container", "proxy"], "ci-frontend-auth-e2e.yaml": ["oidc-e2e"], "ci-frontend-e2ee-e2e.yaml": ["e2ee-e2e"], "cd-images.yaml": ["build"], "ci-db-erd.yaml": ["check"], "ci-sfu-bridge.yaml": ["test"]};
// Kotlin builds use hosted runners for memory and the preinstalled Android SDK.
const hostedJobs = {"cd-images.yaml": ["plan"], "automerge-gate.yaml": ["resolve", "gate"], "pr-review-relay.yaml": ["relay"], "select-tests.yaml": ["select"], "ci-android.yaml": ["android", "publish-main"], "cd-storybook.yaml": ["build"],
  "ci-desktop-tauri.yaml": ["check"],
  "ci-frontend.yaml": ["browser-test"],
  // merge_group には fork の情報がなく、キューに入った PR のコードを self-hosted で動かさない。
  "merge-queue.yaml": ["changes", "all-passed"]};
// macOS 向けのビルドがあるため、GitHub ホストの実行先だけを使うジョブ。PR では動かないワークフローに限る。
// macOS・Windows でビルドを確かめるジョブ。GitHub ホストの実行先を使う (公開リポジトリなので無料)。
const macJobs = {"ci-desktop-tauri.yaml": ["macos"], "ci-ios.yaml": ["check"]};
const windowsJobs = {"ci-desktop-tauri.yaml": ["windows"]};
// Keep timing comparisons on a fixed hosted OS, independent of shared runners.
const benchmarkJobs = {"ci-home-query-budget.yaml": {"query-budget": "ubuntu-24.04"}, "ci-read-state-worker-budget.yaml": {"query-budget": "ubuntu-24.04"}, "ci-phone-query-budget.yaml": {"query-budget": "ubuntu-24.04"}, "ci-search-query-budget.yaml": {"query-budget": "ubuntu-24.04"}, "ci-components-query-budget.yaml": {"query-budget": "ubuntu-24.04"}};
const releaseJobs = {"cd-tui-release.yaml": ["version", "build", "release"],
  "cd-desktop-tauri-release.yaml": ["version", "build", "release"]};
const queued = new Set(['merge-queue.yaml', ...[...readFileSync(new URL('.github/workflows/merge-queue.yaml', root), 'utf8')
  .matchAll(/^    uses: \.\/\.github\/workflows\/([\w-]+\.yaml)$/gm)].map(m => m[1])]);
test('SQL and migration changes require the budget gate in the merge queue', () => {
  const readWorkflow = name => readFileSync(new URL(`.github/workflows/${name}.yaml`, root), 'utf8');
  for (const path of ['backend/src/routes/home.rs', 'infra/db/migrations/example.sql', '.github/workflows/ci-home-query-budget.yaml']) {
    assert.equal(selectWorkflows([path], readWorkflow)['ci-home-query-budget'], true, path);
  }
  assert.equal(selectWorkflows(['docs/example.md'], readWorkflow)['ci-home-query-budget'], false);
  const queue = readWorkflow('merge-queue');
  assert.match(queue, /ci-home-query-budget: \$\{\{ steps\.match\.outputs\.ci-home-query-budget \}\}/);
  const call = queue.split('  ci-home-query-budget:\n')[1].split(/^  [\w-]+:\s*$/m)[0];
  assert.match(call, /if: needs\.changes\.outputs\.ci-home-query-budget == 'true'/);
  assert.match(call, /uses: \.\/\.github\/workflows\/ci-home-query-budget.yaml/);
  assert.match(queue.split('  all-passed:\n')[1], /^      - ci-home-query-budget$/m);
});
test('unread and worker plan budgets are mandatory in the merge queue', () => {
  const readWorkflow = name => readFileSync(new URL(`.github/workflows/${name}.yaml`, root), 'utf8');
  const key = 'ci-read-state-worker-budget';
  for (const path of ['backend/src/routes/read_states.rs', 'backend/src/slack/claim.sql', 'infra/db/schema.sql', `.github/workflows/${key}.yaml`]) {
    assert.equal(selectWorkflows([path], readWorkflow)[key], true, path);
  }
  assert.equal(selectWorkflows(['docs/example.md'], readWorkflow)[key], false);
  const queue = readWorkflow('merge-queue');
  assert.ok(queue.includes('      '+key+': ${{ steps.match.outputs.'+key+' }}'));
  assert.ok(queue.includes(`    if: needs.changes.outputs.${key} == 'true'`));
  assert.ok(queue.includes(`    uses: ./.github/workflows/${key}.yaml`));
  assert.ok(queue.split('  all-passed:\n')[1].includes(`      - ${key}`));
});
test('phone query budgets are mandatory in the merge queue', () => {
  const readWorkflow = name => readFileSync(new URL(`.github/workflows/${name}.yaml`, root), 'utf8');
  const key = 'ci-phone-query-budget';
  for (const path of ['backend/src/routes/phone_sessions.rs', 'backend/src/routes/pbx.rs', 'backend/src/routes/sip.rs', 'backend/tests/phone_query_budget_test.rs', 'infra/db/schema.sql', 'infra/db/compare-phone-work.py', `.github/workflows/${key}.yaml`]) {
    assert.equal(selectWorkflows([path], readWorkflow)[key], true, path);
  }
  assert.equal(selectWorkflows(['docs/example.md'], readWorkflow)[key], false);
  const queue = readWorkflow('merge-queue');
  assert.ok(queue.includes('      '+key+': ${{ steps.match.outputs.'+key+' }}'));
  assert.ok(queue.includes(`    if: needs.changes.outputs.${key} == 'true'`));
  assert.ok(queue.includes(`    uses: ./.github/workflows/${key}.yaml`));
  assert.ok(queue.split('  all-passed:\n')[1].includes(`      - ${key}`));
});
test('search query budgets are mandatory in the merge queue', () => {
  const readWorkflow = name => readFileSync(new URL(`.github/workflows/${name}.yaml`, root), 'utf8');
  const key = 'ci-search-query-budget';
  for (const path of ['backend/src/routes/search.rs', 'backend/src/routes/mod.rs', 'backend/tests/search_query_budget_test.rs', 'infra/db/schema.sql', 'infra/db/compare-search-work.py', 'infra/db/postgres/Dockerfile', `.github/workflows/${key}.yaml`]) {
    assert.equal(selectWorkflows([path], readWorkflow)[key], true, path);
  }
  assert.equal(selectWorkflows(['docs/example.md'], readWorkflow)[key], false);
  const queue = readWorkflow('merge-queue');
  assert.ok(queue.includes('      '+key+': ${{ steps.match.outputs.'+key+' }}'));
  assert.ok(queue.includes(`    if: needs.changes.outputs.${key} == 'true'`));
  assert.ok(queue.includes(`    uses: ./.github/workflows/${key}.yaml`));
  assert.ok(queue.split('  all-passed:\n')[1].includes(`      - ${key}`));
});
test('message component query budgets are mandatory in the merge queue', () => {
  const readWorkflow = name => readFileSync(new URL(`.github/workflows/${name}.yaml`, root), 'utf8');
  const key = 'ci-components-query-budget';
  for (const path of ['backend/src/apps/ui.rs', 'backend/src/apps/delivery.rs', 'backend/src/components/backfill.rs', 'backend/tests/components_query_budget_test.rs', 'infra/db/schema.sql', 'infra/db/compare-components-work.py', `.github/workflows/${key}.yaml`]) {
    assert.equal(selectWorkflows([path], readWorkflow)[key], true, path);
  }
  assert.equal(selectWorkflows(['docs/example.md'], readWorkflow)[key], false);
  const queue = readWorkflow('merge-queue');
  assert.ok(queue.includes('      '+key+': ${{ steps.match.outputs.'+key+' }}'));
  assert.ok(queue.includes(`    if: needs.changes.outputs.${key} == 'true'`));
  assert.ok(queue.includes(`    uses: ./.github/workflows/${key}.yaml`));
  assert.ok(queue.split('  all-passed:\n')[1].includes(`      - ${key}`));
});
function parseNeeds(block) {
  const match = block.match(/^    needs:[ \t]*([^\r\n]*)$/m);
  if (!match) return new Set();
  const rest = match[1].replace(/[ \t]*#.*$/, '').trim();
  if (rest.startsWith('[')) {
    const end = rest.indexOf(']');
    return new Set(
      rest
        .slice(1, end >= 0 ? end : rest.length)
        .split(',')
        .map(s => s.trim())
        .filter(Boolean),
    );
  }
  if (rest) {
    return new Set([rest]);
  }
  const afterNeeds = block.slice(match.index + match[0].length).replace(/^\r?\n/, '').split(/\r?\n/);
  const items = new Set();
  for (const line of afterNeeds) {
    if (!line.trim() || line.trim().startsWith('#')) continue;
    const item = line.match(/^ {4,}-\s+([\w-]+)/);
    if (item) {
      items.add(item[1]);
    } else {
      break;
    }
  }
  return items;
}

function assertSelectorIndependent(block, label) {
  assert.ok(!parseNeeds(block).has('runner'), label);
  assert.doesNotMatch(block, /needs\.runner\./, label);
}

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
      if (benchmarkJobs[file]?.[id]) {
        assert.ok(block.includes(`    runs-on: ${benchmarkJobs[file][id]}\n`), `${file}/${id}: pinned hosted benchmark OS`);
        assertSelectorIndependent(block, `${file}/${id}: selector dependency`);
        assert.match(text, /^  workflow_call:$/m, `${file}: merge queue must call the budget gate`);
        checked++;
        continue;
      }
      if ((releaseJobs[file] || []).includes(id)) {
        assert.doesNotMatch(text.split(/^jobs:\s*$/m)[0], /pull_request/, `${file}: hosted-only workflows must not run on PRs`);
        assert.match(block, /^    runs-on: (?:ubuntu-latest|\$\{\{ matrix\.os \}\})$/m, `${file}/${id}: hosted pool`);
        assertSelectorIndependent(block, `${file}/${id}: selector dependency`);
        checked++;
        continue;
      }
      const docker = (dockerJobs[file] || []).includes(id);
      const large = (file === 'ci-frontend.yaml' && ['check', 'build'].includes(id)) ||
        (file === 'ci-backend.yaml' && id === 'check');
      const hosted = (hostedJobs[file] || []).includes(id);
      const fixed = (fixedJobs[file] || []).includes(id);
      const mac = (macJobs[file] || []).includes(id);
      if (mac) {
        // macOS は GitHub ホストにしかないので、fork の PR でも同じ実行先にする。
        assert.match(block, /^    runs-on: macos-\d+$/m, `${file}/${id}: hosted macOS`);
        assertSelectorIndependent(block, `${file}/${id}: selector dependency`);
        checked++;
        continue;
      }
      const win = (windowsJobs[file] || []).includes(id);
      if (win) {
        // Windows ビルドと Inno Setup も GitHub ホストの実行先を使い、fork の PR でも同じ実行先にする。
        assert.match(block, /^    runs-on: windows-(?:latest|\d+)$/m, `${file}/${id}: hosted Windows`);
        assertSelectorIndependent(block, `${file}/${id}: selector dependency`);
        checked++;
        continue;
      }
      if (hosted) {
        assert.match(block, /^    runs-on: ubuntu-latest$/m, `${file}/${id}: hosted for all PRs`);
        assertSelectorIndependent(block, `${file}/${id}: selector dependency`);
      } else if (fixed) {
        assert.match(block, /runs-on:.*\|\| 'zli-kubecamp-no-docker' \}\}/, `${file}/${id}: fixed pool`);
        assertSelectorIndependent(block, `${file}/${id}: selector dependency`);
      } else {
        assert.ok(block.includes(`needs.runner.outputs.${large ? 'large-runs-on' : docker ? 'docker-runs-on' : 'runs-on'}`), `${file}/${id}: wrong pool`);
        if (docker) {
          assert.ok(block.includes(`docker-runs-on || '"${H}"'`), `${file}/${id}: Docker fallback must be hosted`);
        }
        if (large) {
          // zoo2 に登録される runner は zli-k8s-heavy だけ。select が失敗しても、重いジョブが存在しないラベルで待たないように Hosted にする。
          assert.ok(block.includes(`large-runs-on || '"${H}"'`), `${file}/${id}: large-job fallback must be hosted`);
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
      assertSelectorIndependent(block, `${file}/${id}: selector dependency`);
    }
  }
});

test('parseNeeds detects runner dependencies across scalar, inline, and multiline formats', () => {
  assert.ok(parseNeeds('    needs: runner\n    runs-on: ubuntu-latest').has('runner'));
  assert.ok(parseNeeds('    needs: [version, runner]\n    runs-on: ubuntu-latest').has('runner'));
  assert.ok(parseNeeds('    needs:\n      - version\n      - runner\n    runs-on: ubuntu-latest').has('runner'));
  assert.ok(parseNeeds('    needs:\n    - version\n    - runner\n    runs-on: ubuntu-latest').has('runner'));
  assert.equal(parseNeeds('    needs:\n      - version\n      - build\n    runs-on: ubuntu-latest').has('runner'), false);
});
test('selector host overrides do not override workload priorities', () => {
  assert.match(yaml, /vars\.CI_SELECTOR_RUNNER \|\| vars\.CI_RUNNER/);
  assert.match(yaml, /runs-on:.*github\.event\.repository\.private == false.*&& 'ubuntu-latest' \|\| vars\.CI_SELECTOR_RUNNER/);
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
    if (["cd-images.yaml", "cd-storybook.yaml", "cd-db-erd.yaml", "ci-frontend.yaml"].includes(file)) continue;
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


test('ER diagram deployment only publishes main artifacts without checking out code', () => {
  const text = readFileSync(new URL('.github/workflows/cd-db-erd.yaml', root), 'utf8');
  const build = text.split('  build:\n')[1].split('  deploy:\n')[0];
  const deploy = text.split('  deploy:\n')[1];
  assert.match(build, /needs\.runner\.outputs\.docker-runs-on/);
  assert.match(deploy, /needs: build\n/);
  assert.match(deploy, /runs-on:.*'zli-kubecamp-no-docker'/);
  assert.match(deploy, /if: github\.ref == 'refs\/heads\/main' && github\.event_name != 'pull_request'/);
  assert.match(deploy, /actions\/download-artifact/);
  assert.doesNotMatch(deploy, /actions\/checkout/);
});

test('only workflows that read large-runs-on ask the selector to scan heavy usage', () => {
  assert.match(yaml, /inputs:\n {6}# [^\n]*\n {6}large:\n {8}type: boolean\n {8}default: false/);
  const dir = new URL('.github/workflows/', root);
  for (const file of readdirSync(dir).filter(x => /\.ya?ml$/.test(x) && !x.startsWith('select-runner'))) {
    const text = readFileSync(new URL(file, dir), 'utf8');
    const block = text.split(/^jobs:\s*$/m)[1]?.split(/^  runner:\n/m)[1]?.split(/^  \S/m)[0];
    if (!block) continue;
    const readsLarge = text.includes('needs.runner.outputs.large-runs-on');
    assert.equal(/^    with:\n      large: true$/m.test(block), readsLarge, `${file}: large input matches large-runs-on use`);
  }
});

test('the selector needs no permissions, so reusable-workflow callers need no change', () => {
  // 再利用 workflow は、呼び出し元の権限を増やせない。select に権限を要求すると、呼び出し元の9本が起動前に拒否される
  // （startup_failure）。Actions の読み取りは、権限なしの GITHUB_TOKEN で、公開リポジトリのデータだけを読む。
  assert.doesNotMatch(yaml, /actions: read/);
  assert.match(yaml, /^  select:\n(?:.*\n)*?    permissions: \{\}\n/m);
  // heavy の使用状況は、状態確認用のトークン（PAT）ではなく、ワークフロー自身のトークンで読む。
  // github-script は、別のトークンのクライアントを作る手段（getOctokit）をスクリプトに渡さないので、fetch を直接使う。
  assert.match(yaml, /ACTIONS_READ_TOKEN: \$\{\{ github\.token \}\}/);
  assert.match(script, /authorization: `Bearer \$\{process\.env\.ACTIONS_READ_TOKEN\}`/);
  assert.doesNotMatch(script, /getOctokit\(/);
});
