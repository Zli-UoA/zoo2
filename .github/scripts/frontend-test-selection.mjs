import { execFileSync, spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { escapePattern, readChangedCases } from './browser-test-selection.mjs';
import { classifyTranslationChanges, isFrontendTestInput, isLightweightChange, readChanges, testBaseSha, usesSelectiveTests, workflowChanges } from './test-selection-diff.mjs';
import { selectSourceCases } from './vitest-test-selection.mjs';
export { readChanges } from './test-selection-diff.mjs';

const frontend = fileURLToPath(new URL('../../frontend/', import.meta.url));

export function isRelatedFrontendModule(path, eventName) {
  return /^frontend\/src\/.+\.(ts|tsx)$/.test(path) && !path.endsWith('.d.ts') &&
    (!/^frontend\/src\/(test|routes|sw)\//.test(path) || /\.vrt\.test\.tsx$/.test(path)) &&
    !/^frontend\/src\/lib\/(i18n|pwa|oidc)\//.test(path) &&
    (!path.startsWith('frontend/src/lib/e2ee/') ||
      (eventName === 'pull_request' && !/(?:worker|wasm|runtime)/i.test(path))) &&
    !/(?:^|\/)(?:routeTree\.gen|router|main)\.[^/]+$/.test(path);
}

export function isVitestInput(path) {
  if (path === 'frontend/e2e/mock-idp.ts') return true;
  return path.startsWith('frontend/') && !/^frontend\/(e2e\/|playwright[^/]*\.config\.ts$|scripts\/(?:pwa|avatar-cache|offline-csp)\.test\.mjs$)/.test(path);
}

function isPrVitestInput(change, project) {
  if (['unit', 'browser'].includes(project) && change.path.startsWith('frontend/.storybook/') &&
      !change.path.startsWith('frontend/.storybook/fixtures/')) return false;
  return !isLightweightChange(change) && isFrontendTestInput(change.path) &&
    (isVitestInput(change.path) || !/^(frontend\/|infra\/)/.test(change.path));
}

// Vitest follows imports. Changes outside that graph need the complete suite.
export function selectTests(eventName, event, changes, project) {
  const full = reason => ({ mode: 'full', reason });
  if (!usesSelectiveTests(eventName)) return full('push and manual runs use the full suite');
  if (!testBaseSha(eventName, event)) return full('missing or invalid event base');
  if (!changes?.length) return full('diff unavailable or empty');
  if (eventName === 'pull_request') {
    changes = changes.filter(change => isPrVitestInput(change, project));
  }
  if (!changes.length) return full('no applicable source diff');
  for (const { status, path } of changes) {
    if (!['A', 'M'].includes(status)) return full('deleted, renamed or unrecognized change');
    if (!isRelatedFrontendModule(path, eventName)) return full('configuration, shared setup or non-module resources changed');
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
  for (const file of selection.fullFiles ?? []) args.push('--exclude', file);
  if (selection.patterns) args.push('--testNamePattern', selection.patterns.join('|') || '(?!)');
  else if (selection.titles?.length) args.push('--testNamePattern', selection.titles.map(title => `(?:^| )${escapePattern(title)}$`).join('|'));
  if (project === 'storybook') args.push('--maxWorkers=1', '--fileParallelism=false');
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
  const base = testBaseSha(eventName, event);
  if (usesSelectiveTests(eventName)) {
    try {
      changes = workflowChanges('ci-frontend', readChanges(base, git));
    } catch {
      log('Test Selection: cannot read diff; running the full suite');
    }
  }
  if (eventName === 'pull_request' && changes) {
    changes = classifyTranslationChanges(base, changes, { git })
      .filter(change => isPrVitestInput(change, project));
  }
  const selection = selectSourceCases(project, selectCaseTitles(project, selectTests(eventName, event, changes, project), changes, base, { git, compiler }), changes, base, { git, compiler, log });
  log(`Test Selection (${project}): ${selection.mode}; ${selection.reason}${selection.titles ? `; ${selection.titles.length} case titles` : ''}${selection.patterns ? `; ${selection.patterns.length} cases affected by source dependencies` : ''}`);
  // A real test failure is propagated; it is never retried as a full run.
  const options = { cwd: frontend, stdio: 'inherit' };
  if (!selection.fullFiles?.length || selection.patterns?.length) {
    const result = run('pnpm', testArguments(project, selection), options);
    if (result.status !== 0) return result.status ?? 1;
  }
  if (selection.fullFiles?.length) {
    const args = [...testArguments(project, { mode: 'full' }), ...selection.fullFiles];
    return run('pnpm', args, options).status ?? 1;
  }
  return 0;
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
