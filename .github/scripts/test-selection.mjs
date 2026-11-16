import { appendFileSync, readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';
import { readChanges } from './frontend-test-selection.mjs';
import { testLeg } from './backend-e2e-selection.mjs';

const suites = ['unit', 'browser', 'vrt', 'storybook', 'pwa', 'avatar', 'offline',
  'i18n', 'layout', 'frontend-selection', 'container', 'proxy', 'auth', 'e2ee'];
const specs = {
  'frontend/e2e/i18n.spec.ts': 'i18n', 'frontend/e2e/layout.spec.ts': 'layout',
  'frontend/e2e/frontend-selection.spec.ts': 'frontend-selection',
  'frontend/e2e/csp.spec.ts': 'container', 'frontend/e2e/oidc.spec.ts': 'auth',
  'frontend/e2e/e2ee.spec.ts': 'e2ee',
  'frontend/scripts/pwa.test.mjs': 'pwa',
  'frontend/scripts/avatar-cache.test.mjs': 'avatar',
  'frontend/scripts/offline-csp.test.mjs': 'offline',
};

export function selectSuites(eventName, changes) {
  const full = reason => ({
    reason, frontend: Object.fromEntries(suites.map(s => [s, true])),
    backend: { full: true, targets: [], legs: ['slack', 'core'], run: true, emoji: true },
  });
  if (eventName !== 'pull_request') return full('main, merge queue and manual runs use all suites');
  if (!changes?.length) return full('diff unavailable or empty');
  const frontend = Object.fromEntries(suites.map(s => [s, false]));
  const targets = [];
  for (const { status, path } of changes) {
    if (!['A', 'M'].includes(status)) return full('deletion, rename or unknown status');
    const backend = /^backend\/tests\/([a-zA-Z0-9_]+_test)\.rs$/.exec(path);
    if (backend) { targets.push(backend[1]); continue; }
    if (specs[path]) { frontend[specs[path]] = true; continue; }
    if (/^frontend\/src\/.+\.vrt\.test\.tsx$/.test(path)) { frontend.vrt = true; continue; }
    if (/^frontend\/src\/.+\.browser\.test\.tsx?$/.test(path)) { frontend.browser = true; continue; }
    if (/^frontend\/src\/.+\.test\.ts$/.test(path)) { frontend.unit = true; continue; }
    // Source, fixtures, shared test helpers and configuration can affect any suite.
    return full('application, shared resources or unrecognized test changed');
  }
  const unique = [...new Set(targets)].sort();
  return {
    reason: 'only independent test files changed', frontend,
    backend: {
      full: false, targets: unique, run: unique.length > 0,
      legs: [...new Set(unique.map(testLeg))].sort(), emoji: unique.includes('emoji_service_test'),
    },
  };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  let changes;
  if (process.env.GITHUB_EVENT_NAME === 'pull_request') {
    try {
      const event = JSON.parse(readFileSync(process.env.GITHUB_EVENT_PATH, 'utf8'));
      changes = readChanges(event.pull_request?.base?.sha);
    } catch {
      console.log('Test Selection: diff unavailable; running all suites');
    }
  }
  const selection = selectSuites(process.env.GITHUB_EVENT_NAME, changes);
  console.log(JSON.stringify(selection, null, 2));
  for (const key of ['frontend', 'backend']) {
    appendFileSync(process.env.GITHUB_OUTPUT, `${key}=${JSON.stringify(selection[key])}\n`);
  }
}
