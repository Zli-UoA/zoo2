import assert from 'node:assert/strict';
import { test } from 'node:test';
import { loadTypeScript, parseHunks, selectChangedCases } from './browser-test-selection.mjs';
import { runTests, testArguments } from './frontend-test-selection.mjs';
import { runPlaywright, playwrightArguments } from './playwright-test-selection.mjs';
import { selectNodeTests } from './node-test-selection.mjs';
import { caseDependencies, selectSourceCases } from './vitest-test-selection.mjs';

const compiler = loadTypeScript();
const before = `import { test, expect } from 'runner';
const shared = 42;
test('first [case]', () => {
  expect(shared).toBe(42);
});
test('second', () => {
  expect(shared).toBe(42);
});
`;
const after = before.replace('  expect(shared).toBe(42);', '  expect(shared + 1).toBe(43);');
const hunks = parseHunks('@@ -4 +4 @@\n-old\n+new\n');
const base = 'a'.repeat(40);
const event = { pull_request: { base: { sha: base } } };

test('AST selection narrows an edited body and retains title/line identity', () => {
  const cases = selectChangedCases(before, after, hunks, compiler);
  assert.deepEqual(cases, [{ start: 3, end: 5, title: 'first [case]' }]);
  const args = testArguments('browser', { mode: 'related', files: ['src/example.browser.test.tsx'], titles: cases.map(c => c.title) });
  const pattern = new RegExp(args[args.indexOf('--testNamePattern') + 1]);
  assert.ok(pattern.test('nested suite first [case]'));
  assert.ok(!pattern.test('nested suite second'));
  assert.ok(!pattern.test('first case'));
  assert.ok(playwrightArguments('layout', cases).includes('e2e/layout.spec.ts:3'));
});

test('shared helpers, imports, hooks, removal, ambiguous lines and serial suites use full tests', () => {
  for (const line of [1, 2]) assert.equal(selectChangedCases(before, after, parseHunks(`@@ -${line} +${line} @@`), compiler), null);
  assert.equal(selectChangedCases(before, after, [], compiler), null);
  assert.equal(selectChangedCases(before, after, parseHunks('@@ -3,3 +2,0 @@'), compiler), null);
  assert.equal(selectChangedCases('test("a", () => 1); test("b", () => 2);', 'test("a", () => 1); test("b", () => 3);', parseHunks('@@ -1 +1 @@'), compiler), null);
  const serial = `test.describe.configure({mode:'serial'});\n`;
  assert.equal(selectChangedCases(serial + before, serial + after, parseHunks('@@ -5 +5 @@'), compiler), null);
  assert.equal(selectChangedCases(before, 'invalid ({', hunks, compiler), null);
});

test('parameterized cases use Playwright location, but unknown Vitest titles remain full', () => {
  const source = before.replace("'first [case]'", '`case ${language}`');
  assert.equal(selectChangedCases(source, source, hunks, compiler), null);
  assert.deepEqual(selectChangedCases(source, source, hunks, compiler, { requireTitle: false }), [{ start: 3, end: 5, title: null }]);
});

test('deleting a line in a later duplicate title selects its position, not the first title', () => {
  const source = `describe('one', () => {
  test('same', () => {
    expect(1).toBe(1);
  });
});
describe('two', () => {
  test('same', () => {
    const removed = 1;
    expect(2).toBe(2);
  });
});
`;
  const result = selectChangedCases(source, source.replace('    const removed = 1;\n', ''), parseHunks('@@ -8 +7,0 @@'), compiler, { requireTitle: false });
  assert.deepEqual(result, [{ start: 7, end: 9, title: 'same' }]);
});

const gitFor = (path, diff = '@@ -4 +4 @@') => (command, args) => {
  if (args[0] === 'cat-file') return '';
  if (args[0] === 'show') return args[1].startsWith('HEAD:') ? after : before;
  if (args.includes('--name-status')) return `M\0${path}\0`;
  return diff;
};

test('Vitest executes only changed cases and propagates the actual failure', () => {
  let calls = 0;
  const code = runTests({ project: 'browser', eventName: 'pull_request', event, compiler,
    git: gitFor('frontend/src/example.browser.test.tsx'), log() {},
    run(command, args) { calls++; assert.ok(args.includes('--testNamePattern')); return { status: 1 }; },
  });
  assert.equal(code, 1);
  assert.equal(calls, 1);
});

test('Playwright selects case locations; main and helper edits retain the full suite', () => {
  for (const [eventName, diff, selected] of [['pull_request', '@@ -4 +4 @@', true], ['pull_request', '@@ -2 +2 @@', false], ['merge_group', '@@ -4 +4 @@', true], ['push', '', false]]) {
    const code = runPlaywright({ suite: 'layout', eventName, event: eventName === 'merge_group' ? { merge_group: { base_sha: base } } : event, compiler,
      git: gitFor('frontend/e2e/layout.spec.ts', diff), log() {},
      run(command, args) { assert.equal(args.includes('e2e/layout.spec.ts:3'), selected); return { status: 1 }; },
    });
    assert.equal(code, 1);
  }
  assert.throws(() => playwrightArguments('unknown'));
});

test('Node test dependency analysis follows transitive imports, cycles and re-exports', () => {
  const sources = {
    'test/a.test.ts': 'import {a} from "../src/a.ts";',
    'test/b.test.ts': 'import {b} from "../src/b.ts";',
    'src/a.ts': 'export {value as a} from "./shared.ts";',
    'src/b.ts': 'export const b=1;',
    'src/shared.ts': 'import "./a.ts"; export const value=42;',
  };
  const entries = ['test/a.test.ts', 'test/b.test.ts'];
  const options = { compiler, read: path => sources[path], exists: path => Object.hasOwn(sources, path) };
  assert.deepEqual(selectNodeTests(entries, [{ status: 'M', path: 'src/shared.ts' }], options), ['test/a.test.ts']);
  assert.deepEqual(selectNodeTests(entries, [{ status: 'M', path: 'src/unrelated.ts' }], options), []);
  assert.deepEqual(selectNodeTests(entries, [{ status: 'M', path: 'package.json' }], options), entries);
  sources['src/a.ts'] = 'const path="./shared.ts"; import(path);';
  assert.deepEqual(selectNodeTests(entries, [{ status: 'M', path: 'src/shared.ts' }], options), ['test/a.test.ts']);
  assert.deepEqual(selectNodeTests(entries, [{ status: 'D', path: 'src/shared.ts' }], options), entries);
});

test('source edits select VRT cases through imported components and local helpers', () => {
  const path = 'frontend/src/First.tsx';
  const testPath = 'frontend/src/example.vrt.test.tsx';
  const testSource = `import { First } from './First';
import { Second } from './Second';
import { test } from 'vite-plus/test';
function firstHelper() { return First(); }
test('first component', () => {
  firstHelper();
});
test('second component', () => {
  Second();
});
`;
  const sources = { [path]: 'export function First(){return "new";}', 'frontend/src/Second.tsx': 'export function Second(){return 2;}', [testPath]: testSource };
  const options = { compiler, files: [testPath], read: path => sources[path], exists: path => Object.hasOwn(sources, path), git: () => 'export function First(){return "old";}' };
  const related = { mode: 'related', files: ['src/First.tsx'] };
  assert.deepEqual(selectSourceCases('vrt', related, [{ status: 'M', path }], base, options).patterns, ['(?:^| )first component$']);
  sources[testPath] = testSource.replace("test('first component'", "beforeEach(() => { firstHelper(); });\ntest('first component'");
  assert.deepEqual(selectSourceCases('vrt', related, [{ status: 'M', path }], base, options).patterns, ['(?:^| )first component$', '(?:^| )second component$']);
  sources['frontend/src/effects.ts'] = 'import {First} from "./First"; globalThis.value=First(); export const unused=1;';
  sources[testPath] = 'import {unused} from "./effects";\n' + testSource;
  assert.deepEqual(selectSourceCases('vrt', related, [{ status: 'M', path }], base, options).patterns, ['(?:^| )first component$', '(?:^| )second component$']);
  sources[path] = 'initializeGlobalState(); export function First(){return "new";}';
  assert.equal(selectSourceCases('vrt', related, [{ status: 'M', path }], base, options).patterns, undefined);
  assert.equal(caseDependencies('it.each([1,2])("case %s", () => {});', compiler), null);
  assert.equal(caseDependencies('import {it as spec} from "vitest"; spec("unknown",()=>{});', compiler), null);
});

test('Vite URL resources and frontend aliases resolve to file dependencies; type imports add no runtime edge', () => {
  const sources = { 'frontend/test.ts': 'import {data} from "#/data"; import type {Unused} from "./unused";', 'frontend/src/data.ts': 'import url from "./image.png?url"; export const data=url;', 'frontend/src/image.png': 'image' };
  const options = { compiler, frontendAliases: true, read: path => sources[path], exists: path => Object.hasOwn(sources, path) };
  assert.deepEqual(selectNodeTests(['frontend/test.ts'], [{ status: 'M', path: 'frontend/src/data.ts' }], options), ['frontend/test.ts']);
  assert.deepEqual(selectNodeTests(['frontend/test.ts'], [{ status: 'M', path: 'frontend/unused.ts' }], options), []);
});

test('empty named imports retain their runtime side-effect dependency', () => {
  const sources = { 'test.mjs': 'import {} from "./shared.mjs";', 'shared.mjs': 'globalThis.loaded = true;' };
  assert.deepEqual(selectNodeTests(['test.mjs'], [{ status: 'M', path: 'shared.mjs' }], {
    compiler, read: path => sources[path], exists: path => Object.hasOwn(sources, path),
  }), ['test.mjs']);
});

test('shared assignments retain dependencies and computed object names prevent case narrowing', () => {
  const path = 'frontend/src/First.tsx', testPath = 'frontend/src/example.vrt.test.tsx';
  const sources = {
    [path]: 'export function First(){return "new";}',
    [testPath]: `import {First} from './First';
import {test} from 'vitest';
let renderer;
renderer = First;
test('first', () => {
  renderer();
});
`,
  };
  const related = { mode: 'related', files: ['src/First.tsx'] };
  const options = { compiler, files: [testPath], read: path => sources[path], exists: path => Object.hasOwn(sources, path), git: () => 'export function First(){return "old";}' };
  assert.deepEqual(selectSourceCases('vrt', related, [{ status: 'M', path }], base, options).patterns, ['(?:^| )first$']);
  for (const expression of ['{ [initialize()]: 0 }', '{ [initialize()]() { return 0; } }']) {
    sources[path] = `export const state = ${expression}; export function First(){return "new";}`;
    assert.equal(selectSourceCases('vrt', related, [{ status: 'M', path }], base, options).patterns, undefined);
  }
});

test('independent Playwright spec and fixture edits keep case-level selection', () => {
  const git = (command, args) => args.includes('--name-status')
    ? 'M\0frontend/e2e/layout.spec.ts\0M\0frontend/e2e/frontend-api.mjs\0M\0frontend/e2e/i18n.spec.ts\0'
    : gitFor('frontend/e2e/layout.spec.ts')(command, args);
  const code = runPlaywright({
    suite: 'layout', eventName: 'pull_request', event, compiler, git, log() {},
    run(command, args) {
      assert.ok(args.includes('e2e/layout.spec.ts:3'));
      return { status: 0 };
    },
  });
  assert.equal(code, 0);
});

test('unit test cases can be selected from a pure app module edit', () => {
  const path = 'frontend/src/First.ts';
  const spec = 'frontend/src/example.test.ts';
  const sources = {
    [path]: 'export function first() { return 1; }',
    'frontend/src/Second.ts': 'export function second() { return 2; }',
    'frontend/src/fallback.test.ts': `import { first } from './First'; it.each([1])('shared %s', () => first());`,
    [spec]: `import {first} from './First';
import {second} from './Second';
test('first case', () => first());
test('second case', () => second());`,
  };
  const result = selectSourceCases('unit', { mode: 'related', files: ['src/First.ts'] }, [{ path, status: 'M' }], base, {
    compiler, files: [spec, 'frontend/src/fallback.test.ts'], git: () => 'export function first() { return 0; }',
    read: path => sources[path], exists: path => Object.hasOwn(sources, path),
  });
  assert.deepEqual(result.patterns, ['(?:^| )first case$']);
  assert.deepEqual(result.fullFiles, ['src/fallback.test.ts']);
  assert.ok(testArguments('unit', result).includes('--exclude'));
});

test('case dependencies trace nested destructured aliases back to imports', () => {
  const source = `
    import { first } from './First';
    import { second } from './Second';
    const { run: [fn] } = { run: [first] };
    const { other: { execute } } = { other: { execute: second } };
    test('affected', () => expect(fn()).toBe(1));
    test('independent', () => expect(execute()).toBe(2));
  `;
  assert.deepEqual(caseDependencies(source, compiler)?.map(testCase => testCase.modules), [['./First'], ['./Second']]);
});

test('case dependencies include literal dynamic imports and retain computed imports in full', () => {
  const source = `
    test('affected', async () => (await import('./First')).first());
    test('independent', async () => (await import('./Second')).second());
  `;
  assert.deepEqual(caseDependencies(source, compiler)?.map(testCase => testCase.modules), [['./First'], ['./Second']]);
  assert.equal(caseDependencies(source.replace("'./First'", "name"), compiler), null);
});
