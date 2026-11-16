import { execFileSync, spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = fileURLToPath(new URL('../../', import.meta.url));
const frontend = fileURLToPath(new URL('../../frontend/', import.meta.url));
const shaPattern = /^[a-f0-9]{40}$/;

// Vitest follows imports. Changes outside that graph need the complete suite.
export function selectTests(eventName, event, changes) {
  const full = reason => ({ mode: 'full', reason });
  if (eventName !== 'pull_request') return full('main, merge queue and manual runs use the full suite');
  const base = event?.pull_request?.base?.sha;
  if (!shaPattern.test(base ?? '')) return full('missing or invalid pull request base');
  if (!changes?.length) return full('diff unavailable or empty');
  for (const { status, path } of changes) {
    if (!['A', 'M'].includes(status)) return full('deleted, renamed or unrecognized change');
    if (!/^frontend\/src\/.+\.(ts|tsx)$/.test(path) || path.endsWith('.d.ts')) {
      return full('configuration, dependencies or non-module resources changed');
    }
    if ((/^frontend\/src\/(test|routes|sw)\//.test(path) && !/\.vrt\.test\.tsx$/.test(path)) ||
        /^frontend\/src\/lib\/(i18n|pwa|e2ee|oidc)\//.test(path) ||
        /(?:^|\/)(?:routeTree\.gen|router|main)\.[^/]+$/.test(path)) {
      return full('shared setup, workers, routes or runtime resources changed');
    }
  }
  return { mode: 'related', files: changes.map(({ path }) => path.slice('frontend/'.length)), reason: 'select tests through Vitest import dependencies' };
}

export function readChanges(base, git = execFileSync) {
  if (!shaPattern.test(base ?? '')) throw new Error('Invalid base SHA');
  const options = { cwd: root, encoding: 'utf8', timeout: 30000 };
  try {
    git('git', ['cat-file', '-e', `${base}^{commit}`], { ...options, stdio: 'pipe' });
  } catch {
    git('git', ['fetch', '--no-tags', '--depth=1', 'origin', base], { ...options, stdio: 'pipe' });
  }
  // Disable rename detection so a rename includes a deletion and triggers a full run.
  const fields = git('git', ['diff', '--no-renames', '--name-status', '-z', base, 'HEAD'], options).split('\0');
  if (fields.pop() !== '' || fields.length % 2) throw new Error('Invalid git diff');
  const changes = [];
  for (let i = 0; i < fields.length; i += 2) changes.push({ status: fields[i], path: fields[i + 1] });
  return changes;
}

export function testArguments(project, selection) {
  if (!['unit', 'browser', 'vrt'].includes(project)) throw new Error('Unsupported test project');
  const args = ['exec', 'vp', 'test'];
  // Pass explicit paths instead of --changed: Actions uses shallow checkouts,
  // while Vitest's --changed needs a merge base from the commit history.
  if (selection.mode === 'related') args.push('related', ...selection.files);
  args.push('--project', project, '--run');
  if (selection.mode === 'related') args.push('--passWithNoTests');
  return args;
}

export function runTests({ project, eventName, event, git = execFileSync, run = spawnSync, log = console.log }) {
  let changes;
  if (eventName === 'pull_request') {
    try {
      changes = readChanges(event?.pull_request?.base?.sha, git);
    } catch {
      log('Test Selection: cannot read diff; running the full suite');
    }
  }
  const selection = selectTests(eventName, event, changes);
  log(`Test Selection (${project}): ${selection.mode}; ${selection.reason}`);
  // A real test failure is propagated; it is never retried as a full run.
  const result = run('pnpm', testArguments(project, selection), { cwd: frontend, stdio: 'inherit' });
  return result.status ?? 1;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  let event;
  try {
    event = JSON.parse(readFileSync(process.env.GITHUB_EVENT_PATH, 'utf8'));
  } catch {
    // Local runs and unreadable event payloads use the full suite.
  }
  process.exitCode = runTests({ project: process.argv[2], eventName: process.env.GITHUB_EVENT_NAME, event });
}
