import assert from 'node:assert/strict';
import { classifyTranslationChanges, translationOnlyChange, validateTranslationDictionaries } from './test-selection-diff.mjs';
import { test } from 'node:test';
import { readdirSync } from 'node:fs';
import { selectSuites, shouldVerifySelector } from './test-selection.mjs';
import { cargoCommands, routeTestTargets, rustCaseFilters, slackTests } from './backend-e2e-selection.mjs';
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

test('mixed inputs select the union of their suites without widening unrelated jobs', () => {
  const plan = selectSuites('pull_request', [
    change('frontend/src/components/Button.tsx'),
    change('frontend/src/lib/e2ee/setupController.ts'),
    change('frontend/e2e/layout.spec.ts'),
    change('infra/proxy/stg.conf'),
    change('backend/src/routes/typing.rs'),
    change('backend/tests/threads_test.rs'),
  ]);
  assert.deepEqual(Object.keys(plan.frontend).filter(key => plan.frontend[key]),
    ['unit', 'browser', 'vrt', 'storybook', 'layout', 'proxy']);
  assert.deepEqual(plan.backend.targets, ['app_event_sources_test', 'threads_test', 'typing_test']);
  assert.equal(plan.backend.full, false);
  assert.equal(plan.backend.check, true);
  assert.equal(plan.backend.source, true);
});

test('shared changes broaden only the affected test scope', () => {
  const fixtures = selectSuites('pull_request', [
    change('backend/tests/support/thread_db.rs'),
    change('frontend/src/components/Button.tsx'),
  ]);
  assert.equal(fixtures.backend.full, true);
  assert.equal(fixtures.frontend.browser, true);
  assert.equal(fixtures.frontend.container, false);
  const database = selectSuites('pull_request', [change('infra/db/schema.sql')]);
  assert.equal(database.backend.full, true);
  assert.ok(Object.values(database.frontend).every(value => !value));
  const playwright = selectSuites('pull_request', [change('frontend/e2e/mock-idp.ts')]);
  assert.equal(playwright.frontend.auth, true);
  assert.equal(playwright.frontend.vrt, false);
  const selector = selectSuites('pull_request', [change('.github/scripts/test-selection.mjs')]);
  assert.equal(selector.backend.full, true);
  assert.ok(Object.values(selector.frontend).every(Boolean));
});

test('proxy-only edits do not compile the frontend or Wasm; nginx-only edits select container and offline CSP', () => {
  const proxy = selectSuites('pull_request', [change('infra/proxy/stg.conf')]);
  assert.deepEqual(Object.keys(proxy.frontend).filter(k => proxy.frontend[k]), ['proxy']);
  assert.deepEqual(proxy.frontendBuild, { check: false, build: false, wasm: false });
  const nginx = selectSuites('pull_request', [change('infra/frontend-nginx.conf')]);
  assert.deepEqual(Object.keys(nginx.frontend).filter(k => nginx.frontend[k]), ['offline', 'container']);
  assert.equal(nginx.frontendBuild.wasm, true);
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

test('Android source and translation edits do not select web application tests', () => {
  const locale = change('frontend/src/lib/i18n/locales/ja.json');
  const android = change('frontend-compose/androidApp/src/main/kotlin/zoo/android/Settings.kt');
  const readme = change('frontend/README.md');
  for (const changes of [[locale, android], [locale], [readme]]) {
    const plan = selectSuites('pull_request', changes);
    assert.ok(Object.values(plan.frontend).every(value => !value));
    assert.ok(Object.values(plan.frontendBuild).every(value => !value));
    assert.equal(plan.backend.check, false);
  }
});

test('ordinary web source and test edits do not select the broad Playwright or container suites', () => {
  const selection = selectSuites('pull_request', [
    change('frontend/src/components/Button.tsx'),
    change('frontend/src/components/Button.browser.test.tsx'),
    change('frontend/src/lib/i18n/locales/ja.json'),
    change('frontend-compose/androidApp/src/main/kotlin/zoo/android/Settings.kt'),
  ]);
  assert.deepEqual(Object.keys(selection.frontend).filter(key => selection.frontend[key]), ['unit', 'browser', 'vrt', 'storybook']);
  assert.equal(selection.backend.check, false);
  assert.equal(selection.frontend.catalog, false);
  assert.equal(selectSuites('merge_group', [change('frontend/src/components/Button.tsx')]).frontend.catalog, true);
});

test('translation edits retain broader checks when keys or interpolation variables change', () => {
  const before = '{"setting":"{{count}} items"}';
  assert.equal(translationOnlyChange(before, '{"setting":"Items: {{count}}","added":"New"}', 'en.json'), true);
  assert.equal(translationOnlyChange(before, '{"setting":"{{total}} items"}', 'en.json'), false);
  assert.equal(translationOnlyChange(before, '{}', 'en.json'), false);
  assert.throws(() => translationOnlyChange(before, '{"setting":"{{count"}', 'en.json'), /invalid interpolation/);
  assert.throws(() => translationOnlyChange(before, '{', 'en.json'), SyntaxError);

  const [changeWithRemovedKey] = classifyTranslationChanges('a'.repeat(40), [
    change('frontend/src/lib/i18n/locales/en.json'),
  ], { git: () => before, read: () => '{}' });
  assert.equal(selectSuites('pull_request', [changeWithRemovedKey]).frontend.i18n, true);
});

test('resources consumed outside the Vitest import graph keep their direct checks', () => {
  const worker = selectSuites('pull_request', [change('frontend/public/sw.js')]);
  for (const suite of ['pwa', 'offline', 'avatar', 'container']) assert.equal(worker.frontend[suite], true, suite);
  assert.equal(worker.frontend.catalog, false);
  const story = selectSuites('pull_request', [change('frontend/.storybook/fixtures.ts')]);
  assert.equal(story.frontend.vrt, true);
  assert.equal(story.frontend.catalog, true);
  assert.equal(story.frontend.container, false);
  const compose = selectSuites('pull_request', [change('infra/compose.stack.yaml')]);
  assert.ok(compose.frontend.container && compose.frontend.proxy && compose.backend.deploy);
  assert.ok(!compose.frontend.storybook && !compose.backend.full && !compose.backend.run);
});

test('Rust case filters run exact names and fall back independently per binary', () => {
  const selection = { full: false, targets: ['threads_test', 'typing_test'] };
  const changes = [change('backend/tests/threads_test.rs'), change('backend/tests/typing_test.rs')];
  const input = {
    selection, leg: 'core', base: 'a'.repeat(40), changes,
    git: (command, args) => args[1].split(':')[1],
    run: (command, args, options) => ({
      status: 0,
      stdout: options.input.includes('threads_test') ? 'first\nsecond\n' : '',
    }),
    log() {},
  };
  const filters = rustCaseFilters(input);
  assert.deepEqual(cargoCommands(selection, 'core', [], filters), [
    ['test', '--test', 'typing_test'],
    ['test', '--test', 'threads_test', '--', '--exact', 'first', 'second'],
  ]);
  assert.deepEqual(rustCaseFilters({ ...input, changes: [...changes, change('backend/src/routes/typing.rs')] }), {});
  assert.deepEqual(rustCaseFilters({ ...input, run: () => ({ status: 1 }) }), {});
});

test('Playwright fixture and config ownership does not widen unrelated specs', () => {
  const plan = selectSuites('pull_request', [
    change('frontend/e2e/fixtures/layout.tsx'), change('frontend/playwright.i18n.config.ts'),
  ]);
  assert.deepEqual(Object.keys(plan.frontend).filter(key => plan.frontend[key]), ['i18n', 'layout']);
  for (const path of ['shell.tsx', 'public-profile-fixtures.ts', 'mock-api.ts']) {
    assert.equal(selectSuites('pull_request', [change(`frontend/.storybook/${path}`)]).frontend.browser, true);
  }
  const module = selectSuites('pull_request', [change('frontend/src/components/chat/messages/MessageList.tsx')]);
  assert.equal(module.frontend.layout, true);
  assert.equal(module.frontend.container, false);
});

test('shared fixtures and distribution resources select their direct consumers', () => {
  const plan = selectSuites('pull_request', [
    change('frontend/.storybook/fixtures/sample.eml'),
    change('frontend/.storybook/fixtures.ts'),
    change('frontend/e2e/mock-idp.ts'),
    change('frontend/public/manifest.webmanifest'),
    change('frontend/src/styles.css'),
  ]);
  for (const suite of ['unit', 'browser', 'vrt', 'auth', 'e2ee', 'pwa', 'layout']) {
    assert.equal(plan.frontend[suite], true, suite);
  }
  assert.equal(selectSuites('pull_request', [change('frontend/playwright.layout.config.ts')]).frontendBuild.check, true);
  assert.equal(selectSuites('pull_request', [change('frontend/e2e/layout.spec.ts')]).frontendBuild.check, true);
});

test('lightweight translations reject missing keys, changed placeholders, and empty strings', () => {
  const reference = { 'ja.json': '{"label":"{{count}} 件"}' };
  for (const en of ['{}', '{"label":""}', '{"label":"{{total}} items"}', '{"label":"{{count}} items","other":"Extra"}']) {
    assert.throws(() => validateTranslationDictionaries({ ...reference, 'en.json': en }));
  }
  assert.throws(() => validateTranslationDictionaries({ ...reference, 'en.json': '{"label":"{{ count }} items"}' }), /invalid interpolation/);
  assert.doesNotThrow(() => validateTranslationDictionaries({ ...reference, 'en.json': '{"label":"{{count}} items"}' }));
});

test('lightweight translations use the same English-copy exceptions as the app tests', () => {
  assert.throws(() => validateTranslationDictionaries({
    'ja.json': '{"welcome":{"title":"Welcome to Zoo"}}',
    'en.json': '{"welcome":{"title":"Welcome to Zoo"}}',
  }), /untranslated English copy/);
  assert.doesNotThrow(() => validateTranslationDictionaries({
    'ja.json': '{"home":{"title":"Home"}}',
    'en.json': '{"home":{"title":"Home"}}',
  }));
});

test('changes to the shared translation policy validate the CI selection rules', () => {
  assert.equal(shouldVerifySelector('pull_request', [change('frontend/src/lib/i18n/translation-policy.ts')]), true);
  assert.equal(shouldVerifySelector('pull_request', [change('frontend/src/lib/i18n/locales/ja.json')]), false);
});

test('existing copy edits keep all Web text-sensitive checks before merge', () => {
  const inputs = ['frontend/src/lib/i18n/locales/ja.json'];
  const changes = inputs.map(change);
  const base = 'a'.repeat(40);
  const before = '{"a11y":{"label":"本文へスキップ"}}';
  const edit = after => classifyTranslationChanges(base, changes, { git: () => before, read: () => after });
  const plan = selectSuites('pull_request', edit('{"a11y":{"label":"本文に移動"}}'));
  for (const suite of ['unit', 'browser', 'vrt', 'storybook', 'i18n', 'layout']) assert.equal(plan.frontend[suite], true);
  assert.equal(plan.frontend.container, false);
  const addition = selectSuites('pull_request', edit('{"a11y":{"label":"本文へスキップ","new":"戻る"}}'));
  assert.ok(Object.values(addition.frontend).every(value => !value));
});

test('Slack event documentation embedded in the backend selects deploy validation', () => {
  assert.equal(selectSuites('pull_request', [change('docs/slack-event-coverage.json')]).backend.deploy, true);
});
