import { readFileSync, readdirSync } from 'node:fs';
import { pullRequestPaths, globToRegExp } from './merge-queue-changes.mjs';

export const ignoredWorkflows = new Set(['automerge-gate.yaml', 'cd-images.yaml', 'cd-preview-label.yaml', 'cd-storybook.yaml']);
const basename = path => path?.split('/').at(-1);
const green = new Set(['success', 'skipped', 'neutral']);

export function expectedWorkflows(files, workflows) {
  return workflows.filter(({ path, source }) => {
    if (ignoredWorkflows.has(basename(path)) || !/^  pull_request:/m.test(source)) return false;
    const patterns = pullRequestPaths(source).map(globToRegExp);
    return !patterns.length || files.some(file => patterns.some(p => p.test(file)));
  }).map(w => basename(w.path));
}

export function aggregateChecks(checks, runs, expected = []) {
  const bySuite = new Map(runs.map(r => [r.check_suite_id, r]));
  const newest = new Map();
  for (const run of runs) {
    const key = `${run.path}:${run.event}`;
    if (!newest.has(key) || newest.get(key).id < run.id) newest.set(key, run);
  }
  const activeRuns = [...newest.values()].filter(r => !ignoredWorkflows.has(basename(r.path)));
  const evaluated = checks.filter(check => {
    const run = bySuite.get(check.suite_id);
    if (!run) return true; // Include external check providers.
    if (ignoredWorkflows.has(basename(run.path))) return false;
    const latest = newest.get(`${run.path}:${run.event}`);
    if (latest.id === run.id) return true;
    // Preserve failures if the replacement skipped that job; only an actual
    // replacement result can supersede it. Concurrency cancellations are stale.
    return check.conclusion !== 'cancelled' && !checks.some(other =>
      other.suite_id === latest.check_suite_id && other.name === check.name &&
      other.status === 'completed' && other.conclusion !== 'skipped');
  });
  const missing = expected.filter(path => !activeRuns.some(r => basename(r.path) === path && r.event === 'pull_request'));
  const pending = missing.length > 0 || activeRuns.some(r => r.status !== 'completed' || r.conclusion === null) || evaluated.some(c => c.status !== 'completed' || c.conclusion === null);
  if (pending) return { state: 'pending', description: `Waiting for CI${missing.length ? `: ${missing.join(', ')}` : ''}`.slice(0, 140) };
  const failed = evaluated.some(c => !green.has(c.conclusion)) || activeRuns.some(r => !green.has(r.conclusion));
  return { state: failed ? 'failure' : 'success', description: failed ? 'One or more checks failed' : 'All checks passed' };
}

export async function hasMergeIntent(github, repo, pr) {
  if (pr.auto_merge) return true;
  const reviews = await github.paginate(github.rest.pulls.listReviews, { ...repo, pull_number: pr.number, per_page: 100 });
  const latest = new Map();
  for (const review of reviews) if (review.user?.login && review.state !== 'COMMENTED') latest.set(review.user.login, review.state);
  for (const [username, state] of latest) {
    if (state !== 'APPROVED') continue;
    try {
      const { data } = await github.rest.repos.getCollaboratorPermissionLevel({ ...repo, username });
      if (['admin', 'write'].includes(data.permission) || ['admin', 'maintain', 'write'].includes(data.role_name)) return true;
    } catch (error) { if (error.status !== 404) throw error; }
  }
  return false;
}

export async function evaluatePullRequest({ github, repo, number, workflows, targetUrl }) {
  const { data: pr } = await github.rest.pulls.get({ ...repo, pull_number: number });
  if (pr.state !== 'open') return { state: 'skipped' };
  const sha = pr.head.sha;
  const publish = async result => {
    const { data: current } = await github.rest.pulls.get({ ...repo, pull_number: number });
    if (current.state !== 'open' || current.head.sha !== sha) return { state: 'stale' };
    const { data: status } = await github.rest.repos.getCombinedStatusForRef({ ...repo, ref: sha });
    const previous = status.statuses.find(s => s.context === 'automerge-gate/all-passed');
    if (previous?.state !== result.state) await github.rest.repos.createCommitStatus({
      ...repo, sha, context: 'automerge-gate/all-passed', ...result, target_url: targetUrl,
    });
    return result;
  };
  try {
    if (!await hasMergeIntent(github, repo, pr)) return publish({ state: 'pending', description: 'Waiting for auto-merge or approval from a write-access reviewer' });
    // Revoke an earlier success before reading the new snapshot. No polling or
    // sleeps: incomplete CI leaves a pending status and frees the runner.
    await publish({ state: 'pending', description: 'Evaluating current checks' });
    const [files, runs, suites] = await Promise.all([
      github.paginate(github.rest.pulls.listFiles, { ...repo, pull_number: number, per_page: 100 }),
      github.paginate(github.rest.actions.listWorkflowRunsForRepo, { ...repo, head_sha: sha, per_page: 100 }),
      github.paginate(github.rest.checks.listSuitesForRef, { ...repo, ref: sha, per_page: 100 }),
    ]);
    const checks = (await Promise.all(suites.map(async suite => {
      const list = await github.paginate(github.rest.checks.listForSuite, { ...repo, check_suite_id: suite.id, per_page: 100 });
      return list.map(check => ({ ...check, suite_id: suite.id }));
    }))).flat();
    return publish(aggregateChecks(checks, runs, expectedWorkflows(files.map(f => f.filename), workflows)));
  } catch (error) {
    await publish({ state: 'pending', description: 'Could not read all checks; retry on the next event or manual run' });
    throw error;
  }
}

export default async function gate({ github, context, core }) {
  const payload = context.payload;
  let numbers;
  if (context.eventName === 'workflow_dispatch') {
    const number = Number(payload.inputs?.pull_number);
    if (!Number.isSafeInteger(number) || number < 1) throw new Error('Invalid PR number');
    numbers = [number];
  } else if (payload.pull_request) numbers = [payload.pull_request.number];
  else {
    const sha = payload.workflow_run?.head_sha ?? payload.check_run?.head_sha ?? payload.check_suite?.head_sha;
    if (!sha) return;
    const prs = await github.paginate(github.rest.repos.listPullRequestsAssociatedWithCommit, { ...context.repo, commit_sha: sha, per_page: 100 });
    numbers = prs.filter(pr => pr.state === 'open' && pr.head.sha === sha).map(pr => pr.number);
  }
  const dir = new URL('../workflows/', import.meta.url);
  const workflows = readdirSync(dir).filter(p => /\.ya?ml$/.test(p)).map(path => ({ path, source: readFileSync(new URL(path, dir), 'utf8') }));
  for (const number of numbers) {
    const result = await evaluatePullRequest({ github, repo: context.repo, number, workflows, targetUrl: `${context.serverUrl}/${context.repo.owner}/${context.repo.repo}/actions/runs/${context.runId}` });
    core.info(`PR #${number}: ${result.state}`);
  }
}
