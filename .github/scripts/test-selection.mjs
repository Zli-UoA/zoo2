import { appendFileSync, readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';
import { readChanges } from './test-selection-diff.mjs';
import { testLeg } from './backend-e2e-selection.mjs';
import { selectOtherJobs } from './test-selection-jobs.mjs';
import { workflowJobs, restrictWorkflow } from './workflow-test-selection.mjs';

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
    frontendBuild: { check: true, build: true, wasm: true },
    backend: { full: true, targets: [], legs: ['slack', 'core'], run: true, emoji: true },
  });
  if (eventName !== 'pull_request') return full('main, merge queue and manual runs use all suites');
  if (!changes?.length) return full('diff unavailable or empty');
  // The proxy test uses mock Node servers, not the application or its Wasm.
  if (changes.every(c => ['A', 'M'].includes(c.status) && /^(infra\/proxy\/|infra\/Dockerfile-proxy-deploy)/.test(c.path))) {
    return {
      reason: 'only proxy build/test dependencies changed',
      frontend: Object.fromEntries(suites.map(s => [s, s === 'proxy'])),
      frontendBuild: { check: false, build: false, wasm: false },
      backend: { full: false, targets: [], legs: [], run: false, emoji: false },
    };
  }
  if (changes.every(c => ['A', 'M'].includes(c.status) && /^(infra\/frontend-(nginx\.conf|entrypoint\.sh)|infra\/csp-hashes\.mjs|infra\/Dockerfile-frontend-deploy[^/]*)$/.test(c.path))) {
    return {
      reason: 'only frontend container dependencies changed',
      frontend: Object.fromEntries(suites.map(s => [s, s === 'container'])),
      frontendBuild: { check: false, build: false, wasm: false },
      backend: { full: false, targets: [], legs: [], run: false, emoji: false },
    };
  }
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
    const plan = full('application, shared resources or unrecognized test changed');
    if (changes.every(c => ['A', 'M'].includes(c.status) && c.path.startsWith('backend/tests/'))) {
      plan.frontend = Object.fromEntries(suites.map(s => [s, false]));
      plan.frontendBuild = { check: false, build: false, wasm: false };
    }
    // Ordinary frontend source has no dependency on the proxy's mock servers.
    if (changes.every(c => ['A', 'M'].includes(c.status) && c.path.startsWith('frontend/'))) plan.frontend.proxy = false;
    return plan;
  }
  const unique = [...new Set(targets)].sort();
  return {
    reason: 'only independent test files changed', frontend,
    frontendBuild: { check: true, build: false, wasm: true },
    backend: {
      full: false, targets: unique, run: unique.length > 0,
      legs: [...new Set(unique.map(testLeg))].sort(), emoji: unique.includes('emoji_service_test'),
    },
  };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  let changes;
  let base;
  try {
    const event = JSON.parse(readFileSync(process.env.GITHUB_EVENT_PATH, 'utf8'));
    base = event.pull_request?.base?.sha ?? event.merge_group?.base_sha ?? (process.env.GITHUB_EVENT_NAME === 'push' ? event.before : undefined);
    if (base) changes = readChanges(base);
  } catch {
    console.log('Test Selection: diff unavailable; running all suites');
  }
  let selection = { ...selectSuites(process.env.GITHUB_EVENT_NAME, changes), ...selectOtherJobs(process.env.GITHUB_EVENT_NAME, changes) };
  if (base && changes) {
    const workflow = process.env.TEST_WORKFLOW;
    const jobs = workflowJobs(workflow, changes, { base });
    if (jobs) console.log(`Changed workflow job closure (${workflow}): ${[...jobs].join(', ') || 'no application jobs'}`);
    selection = restrictWorkflow(selection, workflow, jobs);
  }
  selection.desktopTauri ??= { check: true, macos: true, windows: true };
  selection.backend.check ??= true;
  selection.backend.deploy ??= selection.backend.full;
  console.log(JSON.stringify(selection, null, 2));
  for (const key of ['frontend', 'frontendBuild', 'backend', 'clients', 'bots', 'storybookPreview', 'desktopTauri']) {
    appendFileSync(process.env.GITHUB_OUTPUT, `${key}=${JSON.stringify(selection[key])}\n`);
  }
}
