import { appendFileSync, readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';
import { classifyTranslationChanges, isFrontendTestInput, isLightweightChange, readChanges, testBaseSha, usesSelectiveTests, validateChangedTranslations, workflowChanges } from './test-selection-diff.mjs';
import { routeTestTargets, testLeg } from './backend-e2e-selection.mjs';
import { isRelatedFrontendModule, isVitestInput } from './frontend-test-selection.mjs';
import { playwrightOwners } from './playwright-test-selection.mjs';
import { PR_WORKFLOWS } from './merge-queue-changes.mjs';
import { selectOtherJobs } from './test-selection-jobs.mjs';
import { workflowJobs, restrictWorkflow } from './workflow-test-selection.mjs';

const suites = ['unit', 'browser', 'vrt', 'storybook', 'pwa', 'avatar', 'offline',
  'i18n', 'layout', 'frontend-selection', 'container', 'proxy', 'auth', 'e2ee', 'catalog'];
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
  if (!usesSelectiveTests(eventName)) return full('push and manual runs use all suites by default');
  if (!changes?.length) return full('diff unavailable or empty');
  if (eventName === 'pull_request') return selectPullRequest(changes, full);
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

function selectPullRequest(changes, full) {
  const frontend = Object.fromEntries(suites.map(suite => [suite, false]));
  const frontendBuild = { check: false, build: false, wasm: false };
  const backend = { check: false, deploy: false, full: false, source: false, targets: [], legs: [], run: false, emoji: false };
  const targets = new Set();
  const affected = new Map();
  function enable(path, ...selected) {
    for (const suite of selected) frontend[suite] = true;
    affected.set(path, selected.join(', '));
  }

  for (const change of changes) {
    const { path, status } = change;
    if (isLightweightChange(change) || !isFrontendTestInput(path)) continue;
    const edit = ['A', 'M'].includes(status);
    if (path.startsWith('backend/') || /^(infra\/db\/|crates\/zoo-components\/|docs\/slack-event-coverage\.json$)/.test(path) ||
        path === 'frontend/src/lib/emoji/emoji-kitchen.json') {
      backend.check = true;
      const test = /^backend\/tests\/([a-zA-Z0-9_]+_test)\.rs$/.exec(path);
      if (edit && test) targets.add(test[1]);
      else if (edit && Object.hasOwn(routeTestTargets, path)) {
        backend.source = true;
        for (const target of routeTestTargets[path]) targets.add(target);
      } else backend.full = true;
      affected.set(path, backend.full ? 'backend shared inputs' : 'backend integration targets');
      if (path !== 'frontend/src/lib/emoji/emoji-kitchen.json') continue;
    }

    if (/^infra\/(Dockerfile-(backend|migrate)-deploy[^/]*|compose\.stack[^/]*\.yaml)$/.test(path)) {
      backend.deploy = true;
      if (!path.startsWith('infra/compose.stack')) continue;
    }
    if (/^infra\/(proxy\/|Dockerfile-proxy-deploy)/.test(path)) {
      enable(path, 'proxy');
    } else if (/^infra\/(frontend-(nginx\.conf|entrypoint\.sh)|csp-hashes\.mjs|Dockerfile-frontend-deploy[^/]*|compose\.stack[^/]*\.yaml)$/.test(path)) {
      const selected = ['container'];
      if (path.startsWith('infra/compose.stack')) selected.push('proxy');
      if (path === 'infra/frontend-nginx.conf') selected.push('offline');
      enable(path, ...selected);
    } else if (Object.hasOwn(specs, path)) {
      if (/\.tsx?$/.test(path)) frontendBuild.check = true;
      enable(path, specs[path]);
    } else if (/^frontend\/playwright[^/]*\.config\.ts$|^frontend\/e2e\//.test(path)) {
      if (/\.tsx?$/.test(path)) frontendBuild.check = true;
      if (path === 'frontend/e2e/mock-idp.ts') frontend.unit = true;
      const owners = playwrightOwners(path) ?? ['i18n', 'layout', 'frontend-selection', 'csp', 'oidc', 'e2ee'];
      enable(path, ...owners.map(suite => suite === 'csp' ? 'container' : suite === 'oidc' ? 'auth' : suite));
    } else if (path.startsWith('frontend/')) {
      const test = edit && isRelatedFrontendModule(path, 'pull_request');
      if (test && /\.vrt\.test\.tsx$/.test(path)) enable(path, 'vrt');
      else if (test && /\.browser\.test\.tsx?$/.test(path)) enable(path, 'browser');
      else if (test && /\.test\.ts$/.test(path)) enable(path, 'unit');
      else if (path.startsWith('frontend/.storybook/')) {
        const selected = ['vrt', 'storybook', 'catalog'];
        if (path.startsWith('frontend/.storybook/fixtures/')) selected.push('unit', 'browser');
        enable(path, ...selected);
      } else if (/\.stories\.[^/]+$/.test(path)) {
        enable(path, 'vrt', 'storybook', 'catalog');
      } else {
        frontendBuild.build = true;
        const selected = ['unit', 'browser', 'vrt', 'storybook'];
        if (path.startsWith('frontend/src/lib/i18n/')) selected.push('i18n');
        if (path === 'frontend/src/styles.css' || playwrightOwners(path)?.includes('layout')) selected.push('layout');
        if (/^frontend\/(public\/sw(?:-push)?\.js$|src\/(sw\/|lib\/pwa\/))/.test(path)) {
          selected.push('pwa', 'offline', 'container');
        }
        if (/^frontend\/(public\/(sw(?:-push)?\.js|icons\/(favicon-32|badge-96)\.png)$|src\/lib\/pwa\/avatar-cache\.ts$)/.test(path)) {
          selected.push('avatar');
        }
        if (path === 'frontend/public/manifest.webmanifest' || path.startsWith('frontend/public/icons/')) selected.push('pwa');
        if (path.startsWith('frontend/src/lib/offline/')) selected.push('offline');
        if (!isVitestInput(path) || /^frontend\/(package\.json|pnpm-lock\.yaml|vite\.config\.)/.test(path)) {
          selected.push('pwa', 'avatar', 'offline', 'i18n', 'layout', 'frontend-selection', 'container', 'catalog');
        }
        enable(path, ...new Set(selected));
      }
    } else {
      return full(`unclassified shared input: ${path}`);
    }
  }

  frontendBuild.check ||= frontendBuild.build || frontend.unit || frontend.browser || frontend.vrt || frontend.storybook;
  frontendBuild.wasm = frontendBuild.check || ['pwa', 'avatar', 'offline', 'i18n', 'layout', 'frontend-selection', 'catalog'].some(suite => frontend[suite]);
  backend.targets = backend.full ? [] : [...targets].sort();
  backend.legs = backend.full ? ['slack', 'core'] : [...new Set(backend.targets.map(testLeg))].sort();
  backend.run = backend.legs.length > 0;
  backend.emoji = backend.full || targets.has('emoji_service_test');
  return { reason: 'PR suites selected by input', affected: Object.fromEntries(affected), frontend, frontendBuild, backend };
}

export function selectWorkflowPlan(eventName, workflow, changes, base) {
  const relevant = workflowChanges(workflow, changes);
  const otherChanges = eventName === 'pull_request' && relevant?.length
    ? relevant.filter(change => !isLightweightChange(change) && isFrontendTestInput(change.path))
    : relevant;
  const otherJobs = otherChanges?.length === 0 && relevant?.length
    ? { clients: { tui: false }, bots: { bot: false, sdk: false, image: false }, storybookPreview: false }
    : selectOtherJobs(eventName, otherChanges);
  let selection = { ...selectSuites(eventName, relevant), ...otherJobs };
  if (base && changes) {
    const jobs = workflowJobs(workflow, changes, { base });
    if (jobs) console.log(`Changed workflow job closure (${workflow}): ${[...jobs].join(', ') || 'no application jobs'}`);
    selection = restrictWorkflow(selection, workflow, jobs);
  }
  selection.androidMedia = !relevant?.length || relevant.some(({ path }) =>
    path === '.github/workflows/ci-android.yaml' ||
    (path.startsWith('frontend/') && !/^(?:frontend\/src\/(?:android-phone|lib\/(?:sip|call))\/|frontend\/vite\.android-phone\.config\.ts$|frontend\/scripts\/android-phone-smoke\.mjs$)/.test(path)));
  selection.desktopTauri ??= { check: true, macos: true, windows: true };
  if (relevant?.some(({ path }) => path.startsWith('.github/scripts/rust-test-selection/'))) {
    selection.backend.verifyCaseSelector = true;
    selection.backend.check = true;
  }
  selection.backend.check ??= true;
  selection.backend.deploy ??= selection.backend.full;
  return selection;
}

export function selectPrPlans(changes, base) {
  const plans = {};
  for (const workflow of PR_WORKFLOWS) {
    if (changes?.length && !workflowChanges(workflow, changes).length) continue;
    const selection = selectWorkflowPlan('pull_request', workflow, changes, base);
    const { backend, frontend, frontendBuild, clients, bots } = selection;
    if (workflow === 'ci-backend' && (backend.check || (!backend.full && (backend.run || backend.emoji)))) plans[workflow] = { backend };
    if (workflow === 'ci-frontend' && (
      Object.values(frontendBuild).some(Boolean) ||
      Object.entries(frontend).some(([key, enabled]) => !['auth', 'e2ee'].includes(key) && enabled)
    )) plans[workflow] = { frontend, frontendBuild };
    if (workflow === 'ci-tui' && clients.tui) plans[workflow] = { clients };
    if (workflow === 'ci-github-bot' && (bots.bot || bots.sdk)) plans[workflow] = { bots };
  }
  return plans;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const eventName = process.env.GITHUB_EVENT_NAME;
  const workflow = process.env.TEST_WORKFLOW;
  let changes;
  let base;
  try {
    const event = JSON.parse(readFileSync(process.env.GITHUB_EVENT_PATH, 'utf8'));
    base = testBaseSha(eventName, event);
    if (base) changes = readChanges(base);
  } catch {
    console.log('Test Selection: diff unavailable; running all suites');
  }
  if (eventName === 'pull_request' && changes) {
    validateChangedTranslations(changes);
    changes = classifyTranslationChanges(base, changes);
  }
  const sharedPlan = process.env.TEST_PR_BATCH === 'true';
  const selection = sharedPlan ? selectPrPlans(changes, base) : selectWorkflowPlan(eventName, workflow, changes, base);
  const verify = eventName !== 'pull_request' || !changes?.length || changes.some(({ path }) =>
    path.startsWith('.github/') || path === 'desktop/scripts/collect-lucide-icons.mjs');
  const output = process.env.GITHUB_OUTPUT;
  appendFileSync(output, `plan=${JSON.stringify(Object.fromEntries(Object.entries(selection).filter(([key]) => key !== 'affected')))}\nverify=${verify}\n`);
  console.log(JSON.stringify(selection, null, 2));
  if (!sharedPlan) {
    for (const key of ['frontend', 'frontendBuild', 'backend', 'clients', 'bots', 'storybookPreview', 'desktopTauri', 'androidMedia']) {
      appendFileSync(output, `${key}=${JSON.stringify(selection[key])}\n`);
    }
  }
}
