import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readFileSync } from 'node:fs';
import { pathMatcher, selectWorkflows, WORKFLOWS } from './merge-queue-changes.mjs';
import { expectedWorkflows } from './automerge-gate.mjs';
import { workflowChanges } from './test-selection-diff.mjs';
import { changedSections, jobClosure, workflowJobs, restrictWorkflow } from './workflow-test-selection.mjs';

const source = `name: CI
on:
  pull_request:
jobs:
  runner:
    uses: ./runner.yaml
  test-selection:
    uses: ./selection.yaml
  check:
    needs: [runner, test-selection]
    runs-on: ubuntu-latest
    env:
      DEBUG: full
  deploy-image:
    needs: [runner, test-selection]
    runs-on: ubuntu-latest
  e2e:
    needs: [runner, test-selection]
    runs-on: ubuntu-latest
`;

test('a changed consumer selects prerequisites without selecting unrelated consumers', () => {
  const after = source.replace('DEBUG: full', 'DEBUG: line-tables-only');
  const line = source.split('\n').findIndex(s => s.includes('DEBUG:')) + 1;
  const changed = changedSections(source, after, `@@ -${line} +${line} @@`);
  assert.deepEqual([...changed], ['check']);
  assert.deepEqual([...jobClosure(after, changed)].sort(), ['check', 'runner', 'test-selection']);
  assert.deepEqual([...jobClosure(source, new Set(['runner']))].sort(), ['check', 'deploy-image', 'e2e', 'runner', 'test-selection']);
});

test('global settings, structural job changes and unknown diffs retain all jobs', () => {
  assert.equal(changedSections(source, source.replace('name: CI', 'name: New'), '@@ -1 +1 @@'), null);
  assert.equal(changedSections(source, source.replace('  check:', '  renamed:'), '@@ -9 +9 @@'), null);
  assert.equal(changedSections(source, source + 'env:\n  X: 1\n', '@@ -19,0 +20,2 @@'), null);
  assert.equal(changedSections(source, source + '# root comment\n', 'binary diff'), null);
  assert.equal(jobClosure(source, new Set(['missing'])), null);
});

test('removed backend/proxy cache sections do not select an unchanged frontend cache', () => {
  const before = 'services:\n  backend:\n    build: backend\n  proxy:\n    build: proxy\n  frontend:\n    build: frontend\n';
  const after = 'services:\n  frontend:\n    build: frontend\n';
  assert.deepEqual([...changedSections(before, after, '@@ -2,4 +1,0 @@', 'services')], ['backend', 'proxy']);
});

test('CI-only changes are narrowed per workflow; application changes never use this shortcut', () => {
  const path = '.github/workflows/ci-backend.yaml';
  const after = source.replace('DEBUG: full', 'DEBUG: line-tables-only');
  const line = source.split('\n').findIndex(s => s.includes('DEBUG:')) + 1;
  const git = (command, args) => args[0] === 'show' ? (args[1].startsWith('HEAD:') ? after : source) : `@@ -${line} +${line} @@`;
  const jobs = workflowJobs('ci-backend', [{ status: 'M', path }], { base: 'a'.repeat(40), git });
  assert.deepEqual([...jobs].sort(), ['check', 'runner', 'test-selection']);
  const selected = restrictWorkflow({ backend: {} }, 'ci-backend', jobs);
  assert.equal(selected.backend.check, true);
  assert.equal(selected.backend.deploy, false);
  assert.equal(selected.backend.run, false);
  assert.equal(workflowJobs('ci-backend', [{ status: 'M', path: 'backend/src/lib.rs' }]), null);
  assert.equal(workflowJobs('ci-backend', [{ status: 'D', path }]), null);
});

test('Tauri Linux-only edits do not select macOS; frontend proxy-only edits need no Wasm', () => {
  assert.deepEqual(restrictWorkflow({}, 'ci-desktop-tauri', new Set(['check'])).desktopTauri, { check: true, macos: false });
  const plan = restrictWorkflow({ frontend: {} }, 'ci-frontend', new Set(['proxy', 'runner', 'test-selection']));
  assert.equal(plan.frontend.proxy, true);
  assert.equal(plan.frontend.browser, false);
  assert.deepEqual(plan.frontendBuild, { check: false, build: false, wasm: false });
});

test('needs parses its entire multiline block, including comments and later entries', () => {
  const multiline = source.replaceAll('needs: [runner, test-selection]', 'needs:\n      - runner\n      # selection output\n      - test-selection');
  assert.deepEqual([...jobClosure(multiline, new Set(['test-selection']))].sort(), ['check', 'deploy-image', 'e2e', 'runner', 'test-selection']);
  assert.equal(jobClosure(multiline.replace('      - test-selection', '      unexpected: test-selection'), new Set(['runner'])), null);
  assert.equal(jobClosure(multiline.replace('      - test-selection', '      - *unknown'), new Set(['runner'])), null);
});

test('checks deferred on PRs remain wired into the merge queue gate', () => {
  const readWorkflow = name => readFileSync(new URL(`../workflows/${name}.yaml`, import.meta.url), 'utf8');
  const queue = readWorkflow('merge-queue');
  const deferred = {
    'ci-backend': ['deploy-image'],
    'ci-frontend-auth-e2e': ['crypto-wasm'],
    'ci-frontend-e2ee-e2e': ['crypto-wasm'],
    'ci-android': ['android'],
    'ci-desktop-tauri': ['check', 'macos'],
    'ci-home-query-budget': ['query-budget'],
    'ci-components-query-budget': ['query-budget'],
    'ci-phone-query-budget': ['query-budget'],
    'ci-read-state-worker-budget': ['query-budget'],
  };
  for (const [workflow, jobs] of Object.entries(deferred)) {
    const source = readWorkflow(workflow);
    const selected = selectWorkflows([`.github/workflows/${workflow}.yaml`], readWorkflow);
    assert.equal(selected[workflow], true, workflow);
    assert.ok(queue.includes(`uses: ./.github/workflows/${workflow}.yaml`), workflow);
    assert.ok(queue.split('  all-passed:\n')[1].includes(`      - ${workflow}\n`), workflow);

    const blocks = source.split(/^jobs:\n/m)[1].split(/^  (?=[\w-]+:)/m);
    for (const job of jobs) {
      const block = blocks.find(block => block.startsWith(`${job}:\n`));
      assert.match(block, /^    if: (?:>-\n      )?github\.event_name != 'pull_request'/m, `${workflow}/${job}`);
    }
  }
});

test('PR gate and merge queue agree on platform and test ownership', () => {
  const readWorkflow = name => readFileSync(new URL(`../workflows/${name}.yaml`, import.meta.url), 'utf8');
  const definitions = WORKFLOWS.map(name => ({ path: `${name}.yaml`, source: readWorkflow(name) }));
  const scenarios = [
    ['frontend-compose/androidApp/src/main/App.kt', ['ci-android']],
    ['crates/zoo-crypto-ffi/src/lib.rs', ['ci-android']],
    ['frontend/src/android-phone/engine.ts', ['ci-android', 'ci-tui']],
    ['frontend/src/media-preview-entry.tsx', ['ci-android', 'ci-tui']],
    ['frontend/scripts/media-preview-bundle.test.mjs', ['ci-android']],
    ['frontend/e2e/oidc.spec.ts', ['ci-frontend-auth-e2e']],
    ['frontend/e2e/e2ee.spec.ts', ['ci-frontend-e2ee-e2e']],
    ['backend/tests/apns_test.rs', ['ci-backend']],
    ['backend/tests/home_scale_test.rs', ['ci-backend', 'ci-home-query-budget']],
    ['infra/db/erd.sh', ['ci-db-erd']],
    ['infra/db/compare-phone-work.py', ['ci-phone-query-budget']],
    ['sdk/src/e2ee.ts', ['ci-github-bot']],
  ];
  for (const [path, expected] of scenarios) {
    const actual = Object.entries(selectWorkflows([path], readWorkflow)).filter(([, selected]) => selected).map(([name]) => name);
    assert.deepEqual(actual.sort(), expected, path);
    assert.deepEqual(expectedWorkflows([path], definitions).sort(), expected.map(name => `${name}.yaml`), path);
  }
  const selected = selectWorkflows(['frontend/src/lib/sip/controller.ts', 'frontend/src/android-phone/dm-media.browser.test.ts'], readWorkflow);
  assert.equal(selected['ci-frontend'], true);
  assert.equal(selected['ci-android'], true);
  assert.equal(selectWorkflows(['infra/db/schema.sql'], readWorkflow)['ci-components-query-budget'], true);
});

test('a mixed platform diff retains only relevant files for per-workflow selection', () => {
  const web = { status: 'M', path: 'frontend/src/components/Button.tsx' };
  const android = { status: 'D', path: 'frontend/src/android-phone/engine.ts' };
  assert.deepEqual(workflowChanges('ci-frontend', [android, web]), [web]);
  assert.deepEqual(workflowChanges('ci-android', [android, web]), [android]);
});

test('ordered path exclusions support nested and top-level matches and re-inclusion', () => {
  const matches = pathMatcher(['frontend/**', '!frontend/**/*.test.ts', 'frontend/keep.test.ts']);
  assert.equal(matches('frontend/unit.test.ts'), false);
  assert.equal(matches('frontend/src/unit.test.ts'), false);
  assert.equal(matches('frontend/keep.test.ts'), true);
  assert.equal(matches('frontend/src/unit.ts'), true);
});
