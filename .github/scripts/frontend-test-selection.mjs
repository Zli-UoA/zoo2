import { execFileSync, spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { escapePattern, readChangedCases } from './browser-test-selection.mjs';
import { readChanges } from './test-selection-diff.mjs';
import { selectSourceCases } from './vitest-test-selection.mjs';
export { readChanges } from './test-selection-diff.mjs';

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

export function testArguments(project, selection) {
  if (!['unit', 'browser', 'vrt', 'storybook'].includes(project)) throw new Error('Unsupported test project');
  const args = ['exec', 'vp', 'test'];
  // Pass explicit paths instead of --changed: Actions uses shallow checkouts,
  // while Vitest's --changed needs a merge base from the commit history.
  if (selection.mode === 'related') args.push('related', ...selection.files);
  args.push('--project', project, '--run');
  if (selection.mode === 'related') args.push('--passWithNoTests');
  if (selection.patterns) args.push('--testNamePattern', selection.patterns.join('|') || '(?!)');
  else if (selection.titles?.length) args.push('--testNamePattern', selection.titles.map(title => `(?:^| )${escapePattern(title)}$`).join('|'));
  // Headless Chromium runs multiple test files in one browser; serializing avoids rAF starvation on background tabs.
  if (project === 'browser' || project === 'storybook') args.push('--maxWorkers=1', '--fileParallelism=false');
  return args;
}

export function selectCaseTitles(project, selection, changes, base, { git = execFileSync, compiler } = {}) {
  if (selection.mode !== 'related' || project === 'storybook') return selection;
  const belongs = path => {
    if (project === 'vrt') return /\.vrt\.test\.tsx$/.test(path);
    if (project === 'browser') return /\.browser\.test\.tsx?$/.test(path);
    return /\.test\.ts$/.test(path) && !/\.browser\.test\.ts$/.test(path);
  };
  if (!changes.every(c => c.status === 'M' && belongs(c.path))) return selection;
  try {
    const titles = [];
    for (const { path } of changes) {
      const cases = readChangedCases(base, path, { git, compiler });
      if (!cases) return selection;
      titles.push(...cases.map(c => c.title));
    }
    return { ...selection, titles: [...new Set(titles)] };
  } catch {
    return selection;
  }
}

export function runTests({ project, eventName, event, git = execFileSync, run = spawnSync, compiler, log = console.log }) {
  let changes;
  if (eventName === 'pull_request') {
    try {
      changes = readChanges(event?.pull_request?.base?.sha, git);
    } catch {
      log('Test Selection: cannot read diff; running the full suite');
    }
  }
  const base = event?.pull_request?.base?.sha;
  const selection = selectSourceCases(project, selectCaseTitles(project, selectTests(eventName, event, changes), changes, base, { git, compiler }), changes, base, { git, compiler, log });
  log(`Test Selection (${project}): ${selection.mode}; ${selection.reason}${selection.titles ? `; ${selection.titles.length} case titles` : ''}${selection.patterns ? `; ${selection.patterns.length} cases affected by source dependencies` : ''}`);
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
