import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readdirSync } from 'node:fs';
import { selectSuites } from './test-selection.mjs';
import { cargoCommands, routeTestTargets, slackTests } from './backend-e2e-selection.mjs';
import { selectOtherJobs } from './test-selection-jobs.mjs';

const change = path => ({ status: 'M', path });

test('VRT selection matches the tsx-only include; vrt.test.ts belongs to unit', () => {
  for (const status of ['A', 'M']) {
    for (const [extension, expected] of [['ts', 'unit'], ['tsx', 'vrt']]) {
      const plan = selectSuites('pull_request', [{ status, path: `frontend/src/lib/palette.vrt.test.${extension}` }]);
      assert.deepEqual(Object.keys(plan.frontend).filter(k => plan.frontend[k]), [expected]);
    }
  }
});

test('independent frontend tests only select the owning suite', () => {
  for (const [path, expected] of [
    ['frontend/src/lib/time.test.ts', 'unit'],
    ['frontend/src/components/Button.browser.test.tsx', 'browser'],
    ['frontend/src/test/vrt/catalog.vrt.test.tsx', 'vrt'],
    ['frontend/e2e/i18n.spec.ts', 'i18n'], ['frontend/e2e/layout.spec.ts', 'layout'],
    ['frontend/e2e/frontend-selection.spec.ts', 'frontend-selection'],
    ['frontend/e2e/csp.spec.ts', 'container'], ['frontend/e2e/oidc.spec.ts', 'auth'],
    ['frontend/e2e/e2ee.spec.ts', 'e2ee'], ['frontend/scripts/pwa.test.mjs', 'pwa'],
    ['frontend/scripts/avatar-cache.test.mjs', 'avatar'],
    ['frontend/scripts/offline-csp.test.mjs', 'offline'],
  ]) {
    const plan = selectSuites('pull_request', [change(path)]);
    assert.deepEqual(Object.keys(plan.frontend).filter(k => plan.frontend[k]), [expected], path);
    assert.equal(plan.backend.run, false);
  }
});

test('multiple test edits select their union, including backend partitions', () => {
  const plan = selectSuites('pull_request', [
    change('frontend/e2e/oidc.spec.ts'), change('frontend/src/lib/time.test.ts'),
    change('backend/tests/slack_link_test.rs'), change('backend/tests/threads_test.rs'),
    change('backend/tests/slack_link_test.rs'),
  ]);
  assert.equal(plan.frontend.auth, true);
  assert.equal(plan.frontend.unit, true);
  assert.equal(plan.frontend.e2ee, false);
  assert.deepEqual(plan.backend.targets, ['slack_link_test', 'threads_test']);
  assert.deepEqual(plan.backend.legs, ['core', 'slack']);
  assert.equal(plan.backend.full, false);
});

test('merge groups select the union across queued changes', () => {
  const changes = [
    change('frontend/e2e/layout.spec.ts'),
    change('backend/tests/threads_test.rs'),
  ];
  const plan = selectSuites('merge_group', changes);
  assert.deepEqual(Object.keys(plan.frontend).filter(suite => plan.frontend[suite]), ['layout']);
  assert.deepEqual(plan.backend.targets, ['threads_test']);
  assert.deepEqual(plan.backend.legs, ['core']);
  assert.equal(plan.backend.full, false);
  assert.deepEqual(selectOtherJobs('merge_group', changes).clients, { tui: false });
  assert.equal(selectSuites('merge_group', [...changes, change('infra/db/schema.sql')]).backend.full, true);
});

test('Rust tests select only the required partition and special emoji job', () => {
  for (const [name, leg, emoji] of [['threads_test', 'core', false], ['slack_link_test', 'slack', false], ['emoji_service_test', 'core', true]]) {
    const plan = selectSuites('pull_request', [change(`backend/tests/${name}.rs`)]);
    assert.deepEqual(plan.backend.legs, [leg]);
    assert.equal(plan.backend.emoji, emoji);
    assert.ok(Object.values(plan.frontend).every(v => !v));
    assert.deepEqual(cargoCommands(plan.backend, leg, []), [['test', '--test', name]]);
  }
});

test('code, shared helpers, fixtures, unsupported specs and selectors trigger all suites', () => {
  for (const path of [
    'frontend/src/lib/time.ts', 'frontend/src/components/Button.tsx',
    'frontend/src/components/Button.stories.tsx', 'frontend/.storybook/preview.ts',
    'frontend/src/test/browser-setup.ts', 'frontend/e2e/mock-idp.ts',
    'frontend/e2e/smoke.spec.ts', 'frontend/playwright.config.ts',
    'backend/src/routes/messages.rs', 'backend/tests/support/thread_db.rs',
    'backend/tests/display_views.sql', 'backend/Cargo.lock', 'infra/db/schema.sql',
    '.github/scripts/test-selection.mjs', '.github/workflows/select-tests.yaml',
  ]) {
    const plan = selectSuites('pull_request', [change('backend/tests/threads_test.rs'), change(path)]);
    assert.equal(plan.backend.full, true, path);
    if (path.startsWith('backend/tests/')) assert.ok(Object.values(plan.frontend).every(v => !v), path);
    else assert.ok(Object.entries(plan.frontend).every(([key, value]) => value || (key === 'proxy' && path.startsWith('frontend/'))), path);
  }
});

test('proxy-only edits do not compile the frontend or Wasm; nginx-only edits select the container', () => {
  const proxy = selectSuites('pull_request', [change('infra/proxy/stg.conf')]);
  assert.deepEqual(Object.keys(proxy.frontend).filter(k => proxy.frontend[k]), ['proxy']);
  assert.deepEqual(proxy.frontendBuild, { check: false, build: false, wasm: false });
  const nginx = selectSuites('pull_request', [change('infra/frontend-nginx.conf')]);
  assert.deepEqual(Object.keys(nginx.frontend).filter(k => nginx.frontend[k]), ['container']);
  assert.equal(nginx.frontendBuild.wasm, false);
  assert.equal(selectSuites('pull_request', [change('frontend/src/lib/time.test.ts')]).frontendBuild.build, false);
});

test('React edits do not run TUI', () => {
  const plan = selectOtherJobs('pull_request', [change('frontend/src/components/Example.tsx')]);
  assert.deepEqual(plan.clients, { tui: false });
  assert.equal(plan.storybookPreview, true);
  for (const path of ['frontend/src/styles.css', 'frontend/src/lib/i18n/locales/ja.json']) {
    assert.deepEqual(selectOtherJobs('pull_request', [change(path)]).clients, { tui: false });
  }
  assert.deepEqual(selectOtherJobs('pull_request', [change('frontend/src/example.browser.test.tsx')]).clients, { tui: false });
});

test('iOS の変更だけでは Storybook を起動せず、React の変更を含む場合は起動する', () => {
  const ios = ['ios/Sources/App/ZooApp.swift', 'ios/project.yml', 'ios/Resources/Assets.xcassets/icon.png'].map(change);
  assert.equal(selectOtherJobs('pull_request', ios).storybookPreview, false);
  assert.equal(selectOtherJobs('pull_request', [...ios, change('frontend/src/App.tsx')]).storybookPreview, true);
});

test('Rust clients follow package path dependencies and Bot changes do not trigger SDK crypto', () => {
  assert.deepEqual(selectOtherJobs('pull_request', [change('tui/src/main.rs')]).clients, { tui: true });
  assert.deepEqual(selectOtherJobs('pull_request', [change('client-core/src/lib.rs')]).clients, { tui: true });
  assert.deepEqual(selectOtherJobs('pull_request', [change('apps/github-bot/src/cards.ts')]).bots, { bot: true, sdk: false, image: true });
  assert.deepEqual(selectOtherJobs('pull_request', [change('sdk/src/e2ee.ts')]).bots, { bot: false, sdk: true, image: false });
  assert.deepEqual(selectOtherJobs('pull_request', [change('sdk/src/index.ts')]).bots, { bot: true, sdk: true, image: true });
  assert.deepEqual(selectOtherJobs('pull_request', [change('apps/github-bot/test/cards.test.ts')]).bots, { bot: true, sdk: false, image: false });
  assert.equal(selectOtherJobs('pull_request', [change('backend/src/main.rs')]).storybookPreview, false);
});

test('shared component schema versions verify TUI', () => {
  const changes = [change('crates/zoo-components/src/lib.rs')];
  assert.deepEqual(selectOtherJobs('pull_request', changes).clients, { tui: true });
  assert.deepEqual(selectOtherJobs('merge_group', changes).clients, { tui: true });
  assert.equal(selectOtherJobs('pull_request', [change('frontend/src/android-phone/engine.ts')]).storybookPreview, false);
});

test('main, unknown configuration, deletions and missing diffs retain all cross-package jobs', () => {
  for (const [event, changes] of [['push', [change('tui/src/main.rs')]], ['pull_request', undefined], ['pull_request', [{ status: 'D', path: 'tui/src/main.rs' }]], ['pull_request', [change('.github/scripts/test-selection-jobs.mjs')]]]) {
    const plan = selectOtherJobs(event, changes);
    assert.deepEqual(plan.clients, { tui: true });
    assert.deepEqual(plan.bots, { bot: true, sdk: true, image: true });
  }
});

test('push, manual, deletions and missing diff always run all suites', () => {
  for (const name of ['push', 'workflow_dispatch', undefined]) {
    assert.equal(selectSuites(name, [change('frontend/e2e/oidc.spec.ts')]).backend.full, true);
  }
  for (const changes of [undefined, [], [{ status: 'D', path: 'backend/tests/threads_test.rs' }], [{ status: 'R100', path: 'backend/tests/threads_test.rs' }]]) {
    for (const name of ['pull_request', 'merge_group']) assert.equal(selectSuites(name, changes).backend.full, true);
  }
});

test('full Rust partitions cover every current test once and retain ignored ActivityPub checks', () => {
  const files = readdirSync(new URL('../../backend/tests/', import.meta.url));
  for (const name of [...slackTests, ...Object.values(routeTestTargets).flat()]) {
    assert.ok(files.includes(`${name}.rs`), name);
  }
  const commands = ['slack', 'core'].map(leg => cargoCommands({ full: true }, leg, files));
  const targets = commands.flatMap(c => c[0].filter((_, i) => i > 0 && i % 2 === 0));
  assert.deepEqual(targets.sort(), files.filter(f => f.endsWith('.rs')).map(f => f.slice(0, -3)).sort());
  assert.deepEqual(commands[0][1], ['test', '--lib', 'activitypub::', '--', '--include-ignored', '--test-threads=1']);
  assert.equal(commands[1].length, 1);
});

test('empty selected Rust partitions never invoke a bare cargo test, invalid targets fail', () => {
  assert.deepEqual(cargoCommands({ full: false, targets: ['threads_test'] }, 'slack', []), []);
  assert.throws(() => cargoCommands({ full: true }, 'core', []));
  assert.throws(() => cargoCommands({ full: false, targets: ['--help'] }, 'core', []));
  assert.throws(() => cargoCommands({ full: false, targets: [] }, 'wrong', []));
});

test('ordinary React modules use related projects on PRs and full coverage in the queue', () => {
  const changes = [change('frontend/src/components/Button.tsx')];
  const pr = selectSuites('pull_request', changes);
  assert.deepEqual(Object.keys(pr.frontend).filter(key => pr.frontend[key]), ['unit', 'browser', 'vrt', 'storybook']);
  assert.equal(pr.backend.check, false);
  const withTest = [...changes, change('frontend/src/components/Button.browser.test.tsx')];
  assert.deepEqual(selectSuites('pull_request', withTest).frontend, pr.frontend);
  assert.equal(selectSuites('merge_group', changes).frontend.catalog, true);
  assert.equal(selectSuites('pull_request', [...changes, change('frontend/vite.config.ts')]).frontend.catalog, true);
});

test('classified backend routes select dedicated targets and independently edited tests', () => {
  const changes = [
    change('backend/src/routes/apns.rs'),
    change('backend/src/routes/typing.rs'),
    change('backend/tests/channel_pins_test.rs'),
  ];
  const { backend } = selectSuites('pull_request', changes);
  assert.equal(backend.source, true);
  assert.equal(backend.full, false);
  assert.deepEqual(backend.legs, ['core', 'slack']);
  assert.deepEqual(backend.targets, ['apns_test', 'app_event_sources_test', 'channel_pins_test', 'typing_test']);
  assert.equal(selectSuites('merge_group', changes).backend.full, true);
  const shared = selectSuites('pull_request', [...changes, change('backend/src/routes/mod.rs')]);
  assert.equal(shared.backend.full, true);
});
