import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { test } from 'node:test';
import { createHash } from 'node:crypto';
import { aggregateChecks, evaluatePullRequest, expectedWorkflows, hasMergeIntent, ignoredWorkflows, pullRequestWorkflows, resolvePullRequestNumbers } from './automerge-gate.mjs';

const run = (overrides = {}) => ({ id: 10, path: '.github/workflows/ci.yaml', event: 'pull_request', check_suite_id: 1, status: 'completed', conclusion: 'success', ...overrides });
const check = (overrides = {}) => ({ suite_id: 1, name: 'test', status: 'completed', conclusion: 'success', ...overrides });

test('single snapshot stays pending for queued work, missing workflows and null conclusions', () => {
  assert.equal(aggregateChecks([], [run({ status: 'queued', conclusion: null })]).state, 'pending');
  assert.equal(aggregateChecks([check()], [run()], ['not-yet-registered.yaml']).state, 'pending');
  assert.equal(aggregateChecks([check({ status: 'in_progress', conclusion: null })], [run()]).state, 'pending');
  assert.equal(aggregateChecks([check({ conclusion: null })], [run()]).state, 'pending');
});

test('external failures are required; green, neutral and skipped conclusions remain allowed', () => {
  for (const conclusion of ['failure', 'cancelled', 'timed_out', 'action_required', 'unknown']) {
    assert.equal(aggregateChecks([check({ suite_id: 99, conclusion })], [run()]).state, 'failure');
  }
  for (const conclusion of ['success', 'neutral', 'skipped']) assert.equal(aggregateChecks([check({ conclusion })], [run()]).state, 'success');
});

test('the gate and optional deployment workflows never block themselves', () => {
  for (const path of ignoredWorkflows) {
    assert.equal(aggregateChecks([check({ status: 'in_progress', conclusion: null })], [run({ path: `.github/workflows/${path}`, status: 'in_progress' })]).state, 'success');
  }
});

test('replacement workflows supersede cancellations, but skipped jobs cannot erase failures', () => {
  const runs = [run({ status: 'completed', conclusion: 'cancelled' }), run({ id: 20, check_suite_id: 2 })];
  assert.equal(aggregateChecks([check({ conclusion: 'cancelled' }), check({ suite_id: 2 })], runs).state, 'success');
  assert.equal(aggregateChecks([check({ conclusion: 'failure' }), check({ suite_id: 2, conclusion: 'skipped' })], runs).state, 'failure');
  assert.equal(aggregateChecks([check({ conclusion: 'failure' }), check({ suite_id: 2 })], runs).state, 'success');
  assert.equal(aggregateChecks([check({ conclusion: 'cancelled' })], [run({ conclusion: 'cancelled' })]).state, 'failure');
});

test('expected workflows come from PR paths before jobs are registered', () => {
  const workflow = { path: 'ci.yaml', source: 'name: CI\non:\n  pull_request:\n    paths:\n      - "frontend/**"\n' };
  assert.deepEqual(expectedWorkflows(['frontend/src/app.ts'], [workflow]), ['ci.yaml']);
  assert.deepEqual(expectedWorkflows(['docs/readme.md'], [workflow]), []);
  assert.deepEqual(expectedWorkflows(['docs/readme.md'], [{ path: 'always.yaml', source: 'on:\n  pull_request:\n' }]), ['always.yaml']);
});

function fixture({ status = 'completed', conclusion = 'success', reviews = [], permission = 'read', auto = true, failApi = false } = {}) {
  const writes = [];
  const pr = { state: 'open', number: 7, head: { sha: 'a'.repeat(40) }, auto_merge: auto ? {} : null };
  let previous = 'success';
  const github = { rest: {
    pulls: { get: async () => ({ data: pr }), listFiles: async () => ({ data: [{ filename: 'frontend/a.ts' }] }), listReviews: async () => ({ data: reviews }) },
    actions: { listWorkflowRunsForRepo: async () => { if (failApi) throw new Error('API unavailable'); return { data: [run({ status, conclusion })] }; } },
    checks: { listSuitesForRef: async () => ({ data: [{ id: 1 }] }), listForSuite: async () => ({ data: [check({ status, conclusion })] }) },
    repos: {
      getCollaboratorPermissionLevel: async () => ({ data: { permission } }),
      getCombinedStatusForRef: async () => ({ data: { statuses: [{ context: 'automerge-gate/all-passed', state: previous }] } }),
      createCommitStatus: async params => { writes.push(params); previous = params.state; },
    },
  }, paginate: async (method, params) => (await method(params)).data };
  return { github, pr, writes, repo: { owner: 'org', repo: 'repo' }, number: 7, workflows: [], targetUrl: 'https://example.test/run' };
}

test('pending CI writes pending and returns after one snapshot instead of polling', async () => {
  const f = fixture({ status: 'in_progress', conclusion: null });
  const result = await evaluatePullRequest(f);
  assert.equal(result.state, 'pending');
  assert.deepEqual(f.writes.map(w => w.state), ['pending']);
});

test('completion events publish success or failure and API errors cannot leave green', async () => {
  for (const conclusion of ['success', 'failure']) {
    const f = fixture({ conclusion });
    assert.equal((await evaluatePullRequest(f)).state, conclusion);
    assert.deepEqual(f.writes.map(w => w.state), ['pending', conclusion]);
  }
  const f = fixture({ failApi: true });
  await assert.rejects(evaluatePullRequest(f), /API unavailable/);
  assert.deepEqual(f.writes.map(w => w.state), ['pending']);
});

test('approval intent requires write access and honors dismissed/changes-requested reviews', async () => {
  for (const [reviews, permission, expected] of [
    [[{ user: { login: 'alice' }, state: 'APPROVED' }], 'read', false],
    [[{ user: { login: 'alice' }, state: 'APPROVED' }], 'write', true],
    [[{ user: { login: 'alice' }, state: 'APPROVED' }, { user: { login: 'alice' }, state: 'COMMENTED' }], 'write', true],
    [[{ user: { login: 'alice' }, state: 'APPROVED' }, { user: { login: 'alice' }, state: 'DISMISSED' }], 'write', false],
    [[{ user: { login: 'alice' }, state: 'APPROVED' }, { user: { login: 'alice' }, state: 'CHANGES_REQUESTED' }], 'write', false],
  ]) {
    const f = fixture({ reviews, permission, auto: false });
    assert.equal(await hasMergeIntent(f.github, f.repo, f.pr), expected);
  }
});

test('absent merge intent revokes success; closed PRs and changed heads cannot become green', async () => {
  const f = fixture({ auto: false });
  assert.equal((await evaluatePullRequest(f)).state, 'pending');
  const closed = fixture(); closed.pr.state = 'closed';
  assert.equal((await evaluatePullRequest(closed)).state, 'skipped');
  assert.deepEqual(closed.writes, []);
  const stale = fixture();
  let calls = 0;
  stale.github.rest.pulls.get = async () => ({ data: { ...stale.pr, head: { sha: calls++ ? 'b'.repeat(40) : stale.pr.head.sha } } });
  assert.equal((await evaluatePullRequest(stale)).state, 'stale');
  assert.deepEqual(stale.writes, []);
});

test('workflow completion subscription covers every non-optional PR workflow and excludes itself', () => {
  const dir = new URL('../workflows/', import.meta.url);
  const gate = readFileSync(new URL('automerge-gate.yaml', dir), 'utf8');
  const subscription = gate.split('  workflow_run:\n')[1].split('  check_run:')[0];
  for (const path of readdirSync(dir).filter(p => p.endsWith('.yaml') && !ignoredWorkflows.has(p))) {
    const source = readFileSync(new URL(path, dir), 'utf8');
    if (!/^  pull_request:/m.test(source)) continue;
    const name = source.match(/^name: (.+)$/m)[1].replace(/^["']|["']$/g, '');
    assert.ok(subscription.includes(JSON.stringify(name)), path);
  }
  assert.ok(!subscription.includes('"automerge-gate"'));
  assert.ok(!gate.includes('pkgdeps/automerge-gate'));
  assert.match(gate, /timeout-minutes: 5/);
  assert.match(gate, /group: gate-\$\{\{ github.repository \}\}-pr-\$\{\{ matrix.number \}\}/);
  assert.match(gate, /cancel-in-progress: false/);
  assert.match(gate, /check_suite:\n    types: \[completed\]/);
  assert.ok(subscription.includes('"PR review relay"'));
  const relay = readFileSync(new URL('pr-review-relay.yaml', dir), 'utf8');
  assert.match(relay, /permissions: \{\}/);
  assert.match(relay, /group: pr-review-relay-\$\{\{ github.event.pull_request.number \}\}/);
  assert.doesNotMatch(relay, /^\s+uses:.*checkout/m);
});

test('expected workflows use PR merge definitions as data, including deletion and edits', async () => {
  const source = 'on:\n  pull_request:\n    paths:\n      - "frontend/**"\n';
  const hash = createHash('sha1').update(`blob ${Buffer.byteLength(source)}\0`).update(source).digest('hex');
  const local = [{ path: 'deleted.yaml', source }, { path: 'retained.yaml', source }];
  let fetched = 0;
  const github = { rest: { repos: { getContent: async params => {
    assert.equal(params.ref, 'f'.repeat(40));
    return { data: [
      { name: 'retained.yaml', path: '.github/workflows/retained.yaml', type: 'file', sha: hash },
      { name: 'added.yaml', path: '.github/workflows/added.yaml', type: 'file', sha: 'new' },
    ] };
  } }, git: { getBlob: async () => {
    fetched++;
    return { data: { encoding: 'base64', content: Buffer.from('on:\n  pull_request:\n# globalThis.mustNeverExecute = true').toString('base64') } };
  } } } };
  const definitions = await pullRequestWorkflows(github, {}, { merge_commit_sha: 'f'.repeat(40) }, local);
  assert.deepEqual(expectedWorkflows(['frontend/a.ts'], definitions), ['retained.yaml', 'added.yaml']);
  assert.equal(fetched, 1);
  assert.equal(globalThis.mustNeverExecute, undefined);
  await assert.rejects(pullRequestWorkflows(github, {}, {}, local), /merge result/);
});

test('manual and workflow/review relay events resolve to the same PR identifier', async () => {
  const pr = { number: 7, state: 'open', head: { sha: 'a'.repeat(40) }, merge_commit_sha: 'b'.repeat(40) };
  const github = { rest: { pulls: { get: async () => ({ data: pr }) } } };
  assert.deepEqual(await resolvePullRequestNumbers({ github, context: { eventName: 'workflow_dispatch', payload: { inputs: { pull_number: '7' } } } }), [7]);
  assert.deepEqual(await resolvePullRequestNumbers({ github, context: { eventName: 'workflow_run', repo: {}, payload: { workflow_run: { name: 'PR review relay', head_sha: pr.merge_commit_sha, pull_requests: [{ number: 7 }] } } } }), [7]);
  assert.deepEqual(await resolvePullRequestNumbers({ github, context: { eventName: 'workflow_run', repo: {}, payload: { workflow_run: { name: 'PR review relay', head_sha: 'c'.repeat(40), pull_requests: [{ number: 7 }] } } } }), [7]);
  assert.deepEqual(await resolvePullRequestNumbers({ github, context: { eventName: 'workflow_run', repo: {}, payload: { workflow_run: { head_sha: 'c'.repeat(40), pull_requests: [{ number: 7 }] } } } }), []);
});

test('missing workflow directory is empty only after the merge ref is verified', async () => {
  const ref = 'f'.repeat(40);
  let unresolved = true;
  const github = { rest: { repos: {
    getContent: async () => { throw Object.assign(new Error('Not found'), { status: 404 }); },
    getCommit: async params => {
      assert.equal(params.ref, ref);
      if (unresolved) throw Object.assign(new Error('Unknown merge ref'), { status: 404 });
      return { data: { sha: ref } };
    },
  } } };
  await assert.rejects(pullRequestWorkflows(github, {}, { merge_commit_sha: ref }), /Unknown merge ref/);
  const f = fixture();
  f.loadWorkflows = () => pullRequestWorkflows(github, {}, { merge_commit_sha: ref });
  await assert.rejects(evaluatePullRequest(f), /Unknown merge ref/);
  assert.ok(f.writes.every(write => write.state === 'pending'));
  unresolved = false;
  assert.deepEqual(await pullRequestWorkflows(github, {}, { merge_commit_sha: ref }), []);
});

test('fork review relay without linked PRs re-evaluates current reviews after a head change', async () => {
  const pr = { number: 7, state: 'open', head: { sha: 'a'.repeat(40) } };
  const github = { rest: { pulls: { get: async () => ({ data: pr }) }, repos: { listPullRequestsAssociatedWithCommit: 'associated' } },
    paginate: async (method, params) => {
      assert.equal(method, 'associated');
      assert.equal(params.commit_sha, 'c'.repeat(40));
      return [{ number: 7 }];
    } };
  const context = { eventName: 'workflow_run', repo: {}, payload: { workflow_run: { name: 'PR review relay', head_sha: 'c'.repeat(40), pull_requests: [] } } };
  assert.deepEqual(await resolvePullRequestNumbers({ github, context }), [7]);
  pr.state = 'closed';
  assert.deepEqual(await resolvePullRequestNumbers({ github, context }), []);
});
