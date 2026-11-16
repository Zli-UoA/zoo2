import assert from 'node:assert/strict';
import { test } from 'node:test';
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
