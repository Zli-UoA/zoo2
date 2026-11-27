import { execFileSync, spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { readChanges, testBaseSha, usesSelectiveTests, workflowChanges } from './test-selection-diff.mjs';
import { readChangedCases } from './browser-test-selection.mjs';

const frontend = fileURLToPath(new URL('../../frontend/', import.meta.url));
export const playwrightSuites = {
  layout: 'layout', i18n: 'i18n', 'frontend-selection': 'frontend-selection',
  csp: 'csp', oidc: 'oidc', e2ee: 'e2ee',
};

export function playwrightArguments(suite, cases) {
  if (!Object.hasOwn(playwrightSuites, suite)) throw new Error('Unsupported Playwright suite');
  const args = ['exec', 'playwright', 'test', '--config', `playwright.${playwrightSuites[suite]}.config.ts`];
  if (cases?.length) args.push(...cases.map(c => `e2e/${suite}.spec.ts:${c.start}`));
  return args;
}

export function runPlaywright({ suite, eventName, event, git = execFileSync, run = spawnSync, compiler, log = console.log }) {
  let cases;
  const path = `frontend/e2e/${suite}.spec.ts`;
  if (usesSelectiveTests(eventName)) {
    try {
      const base = testBaseSha(eventName, event);
      const workflow = ['oidc', 'e2ee'].includes(suite)
        ? `ci-frontend-${suite === 'oidc' ? 'auth' : 'e2ee'}-e2e`
        : 'ci-frontend';
      const changes = workflowChanges(workflow, readChanges(base, git));
      if (changes.length === 1 && changes[0].status === 'M' && changes[0].path === path) {
        cases = readChangedCases(base, path, { git, compiler, requireTitle: false });
      }
    } catch {
      log('Playwright Test Selection: diff or case analysis unavailable; running full suite');
    }
  }
  log(`Playwright Test Selection (${suite}): ${cases?.length ? `cases at lines ${cases.map(c => c.start).join(', ')}` : 'full suite'}`);
  const result = run('pnpm', playwrightArguments(suite, cases), { cwd: frontend, stdio: 'inherit' });
  return result.status ?? 1;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  let event;
  try { event = JSON.parse(readFileSync(process.env.GITHUB_EVENT_PATH, 'utf8')); } catch { /* Full local run. */ }
  process.exitCode = runPlaywright({ suite: process.argv[2], eventName: process.env.GITHUB_EVENT_NAME, event });
}
