import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readdirSync } from 'node:fs';
import { selectSuites } from './test-selection.mjs';
import { cargoCommands, slackTests } from './backend-e2e-selection.mjs';

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
    assert.ok(Object.values(plan.frontend).every(Boolean), path);
  }
});

test('main, queue, manual, deletions and missing diff always run all suites', () => {
  for (const name of ['push', 'merge_group', 'workflow_dispatch', undefined]) {
    assert.equal(selectSuites(name, [change('frontend/e2e/oidc.spec.ts')]).backend.full, true);
  }
  for (const changes of [undefined, [], [{ status: 'D', path: 'backend/tests/threads_test.rs' }], [{ status: 'R100', path: 'backend/tests/threads_test.rs' }]]) {
    assert.equal(selectSuites('pull_request', changes).backend.full, true);
  }
});

test('full Rust partitions cover every current test once and retain ignored ActivityPub checks', () => {
  const files = readdirSync(new URL('../../backend/tests/', import.meta.url));
  for (const name of slackTests) assert.ok(files.includes(`${name}.rs`), name);
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
