import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readChanges, runTests, selectTests, testArguments } from './frontend-test-selection.mjs';

const base = 'a'.repeat(40);
const event = { pull_request: { base: { sha: base } } };
const changes = [{ status: 'M', path: 'frontend/src/lib/time.ts' }];

test('source and test additions or edits select tests through imports', () => {
  for (const path of ['frontend/src/lib/time.ts', 'frontend/src/lib/time.test.ts', 'frontend/src/components/Button.tsx']) {
    for (const status of ['A', 'M']) {
      assert.equal(selectTests('pull_request', event, [{ status, path }]).mode, 'related');
    }
  }
});

test('main, merge queue and manual runs always run the complete suite', () => {
  for (const name of ['push', 'merge_group', 'workflow_dispatch', undefined]) {
    assert.equal(selectTests(name, event, changes).mode, 'full');
  }
});

test('configuration, shared setup and resources fall back to full', () => {
  for (const path of [
    'frontend/package.json', 'frontend/pnpm-lock.yaml', 'frontend/vite.config.ts',
    'frontend/tsconfig.json', 'frontend/.storybook/preview.ts',
    'frontend/src/test/browser-setup.ts', 'frontend/src/routes/index.tsx',
    'frontend/src/sw/push.ts', 'frontend/src/lib/i18n/locale.ts',
    'frontend/src/lib/pwa/push.ts', 'frontend/src/lib/e2ee/worker.ts',
    'frontend/src/lib/oidc/auth-worker.ts', 'frontend/src/routeTree.gen.ts',
    'frontend/src/router.tsx', 'frontend/src/main.tsx', 'frontend/src/vite-env.d.ts',
    'frontend/src/locales/ja.json', 'frontend/src/styles.css', 'crates/zoo-crypto/src/lib.rs',
    '.github/scripts/frontend-test-selection.mjs',
  ]) {
    assert.equal(selectTests('pull_request', event, [...changes, { status: 'M', path }]).mode, 'full', path);
  }
});

test('missing diff, invalid base and deletions never skip tests', () => {
  for (const changed of [undefined, [], [{ ...changes[0], status: 'D' }], [{ ...changes[0], status: 'R100' }]]) {
    assert.equal(selectTests('pull_request', event, changed).mode, 'full');
  }
  for (const payload of [undefined, {}, { pull_request: { base: { sha: '--help' } } }]) {
    assert.equal(selectTests('pull_request', payload, changes).mode, 'full');
  }
});

test('diff includes the complete PR and preserves paths without shell interpolation', () => {
  const calls = [];
  const changed = readChanges(base, (command, args, options) => {
    calls.push({ command, args, options });
    if (args[0] === 'cat-file') throw new Error('base not in shallow checkout');
    return args[0] === 'diff' ? 'M\0frontend/src/lib/a b.ts\0D\0frontend/src/lib/old.ts\0' : '';
  });
  assert.deepEqual(changed, [{ status: 'M', path: 'frontend/src/lib/a b.ts' }, { status: 'D', path: 'frontend/src/lib/old.ts' }]);
  assert.deepEqual(calls.map(c => c.args), [
    ['cat-file', '-e', `${base}^{commit}`],
    ['fetch', '--no-tags', '--depth=1', 'origin', base],
    ['diff', '--no-renames', '--name-status', '-z', base, 'HEAD'],
  ]);
  assert.throws(() => readChanges('--help'));
  assert.throws(() => readChanges(base, () => 'malformed'));
});

test('only selected runs may succeed when no related tests exist', () => {
  const selection = selectTests('pull_request', event, changes);
  for (const project of ['unit', 'browser', 'vrt']) {
    assert.deepEqual(testArguments(project, selection), ['exec', 'vp', 'test', 'related', 'src/lib/time.ts', '--project', project, '--run', '--passWithNoTests']);
    assert.deepEqual(testArguments(project, { mode: 'full' }), ['exec', 'vp', 'test', '--project', project, '--run']);
  }
  assert.throws(() => testArguments('storybook', selection));
});

test('fetch failure runs all tests, and test failures or signals remain failures', () => {
  for (const status of [0, 1, null]) {
    let count = 0;
    const code = runTests({
      project: 'unit', eventName: 'pull_request', event,
      git() { throw new Error('fetch failed'); }, log() {},
      run(command, args) {
        count++;
        assert.equal(command, 'pnpm');
        assert.ok(!args.includes('related'));
        return { status };
      },
    });
    assert.equal(count, 1);
    assert.equal(code, status ?? 1);
  }
});

test('main does not fetch history and a selected test failure is not retried', () => {
  for (const eventName of ['push', 'pull_request']) {
    let runs = 0;
    const code = runTests({
      project: 'browser', eventName, event, log() {},
      git(command, args) {
        assert.equal(eventName, 'pull_request');
        return args[0] === 'diff' ? 'M\0frontend/src/lib/time.ts\0' : '';
      },
      run(command, args) {
        runs++;
        assert.equal(args.includes('related'), eventName === 'pull_request');
        return { status: 1 };
      },
    });
    assert.equal(code, 1);
    assert.equal(runs, 1);
  }
});
