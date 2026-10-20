import {test} from 'node:test';
import assert from 'node:assert/strict';
import {changedComponents} from './image-plan.mjs';
import {candidates, referencedDigests, ignoreMissing} from './image-cleanup.mjs';
const sha = n => n.toString(16).padStart(40, '0');
const version = (id, tags, age = id) => ({id, name: `sha256:${sha(id)}`, created_at: new Date(Date.now() - age * 86400000).toISOString(), metadata: {container: {tags}}});
test('component filters and common inputs', () => {
  for (const c of ['frontend', 'backend', 'migrate']) {
    assert.deepEqual(changedComponents([`infra/Dockerfile-${c}-deploy.dockerignore`]), [c]);
  }
  assert.deepEqual(changedComponents(['frontend/src/a.ts']), ['frontend']);
  assert.deepEqual(changedComponents(['backend/Cargo.lock']), ['backend']);
  assert.deepEqual(changedComponents(['infra/db/migrations/a.sql']), ['migrate']);
  assert.equal(changedComponents(['.github/scripts/image-plan.mjs']).length, 3);
  assert.deepEqual(changedComponents(['README.md']), []);
});
test('retain 15 main and preview versions, two open PR generations', () => {
  const versions = Array.from({length: 17}, (_, i) => version(i + 1, [sha(i + 1)]));
  assert.deepEqual(candidates(versions, new Set()).map(v => v.id), [16, 17]);
  assert.deepEqual(candidates(versions.map(v => version(v.id, [`preview-${sha(v.id)}`])), new Set()).map(v => v.id), [16, 17]);
  assert.deepEqual(candidates([1, 2, 3].map(i => version(i, [`pr-7-${sha(i)}`])), new Set([7])).map(v => v.id), [3]);
});
test('closed PR, exact PR boundary, protected and unknown tags', () => {
  const versions = [version(1, [`pr-7-${sha(1)}`]), version(2, [`pr-70-${sha(2)}`]), version(3, [`pr-7-${sha(3)}`, 'latest']), version(4, ['custom']), version(5, ['preview'])];
  assert.deepEqual(candidates(versions, new Set(), {closedPR: 7}).map(v => v.id), [1]);
  assert.deepEqual(candidates(versions, new Set([7]), {closedPR: 7}), []);
  assert.deepEqual(candidates(versions, new Set()).map(v => v.id), [1, 2]);
});
test('multiple tags preserve version if any tag is retained', () => {
  const versions = Array.from({length: 17}, (_, i) => version(i + 1, [sha(i + 1)]));
  versions[16].metadata.container.tags.push(`pr-9-${sha(17)}`);
  assert.deepEqual(candidates(versions, new Set([9])).map(v => v.id), [16]);
});
test('untagged grace period', () => {
  assert.deepEqual(candidates([version(1, [], 0.5), version(2, [], 2)], new Set()).map(v => v.id), [2]);
});
test('OCI indexes recursively protect shared platform and attestation manifests', async () => {
  const manifests = {root: {manifests: [{digest: 'platform'}, {digest: 'attestation'}]}, platform: {}, attestation: {}};
  assert.deepEqual([...await referencedDigests(['root'], async d => manifests[d])], ['root', 'platform', 'attestation']);
  await assert.rejects(referencedDigests(['root'], async () => {throw new Error('403');}));
});
test('only missing versions are ignored', async () => {
  assert.equal(await ignoreMissing(async () => {throw {status: 404};}), null);
  await assert.rejects(ignoreMissing(async () => {throw new Error('403');}));
});

import plan from './image-plan.mjs';
function planFixture({files = [], fork = false, state = 'open', currentSha = sha(1)} = {}) {
  const outputs = {};
  let requestedPages = false;
  const core = {setOutput: (key, value) => {outputs[key] = value;}, summary: {addHeading() {return this;}, addCodeBlock() {return this;}, async write() {}}};
  const context = {repo: {owner: 'Zli-UoA', repo: 'zoo2'}, sha: sha(99), payload: {pull_request: {number: 7, head: {sha: sha(1), repo: {full_name: fork ? 'someone/zoo2' : 'Zli-UoA/zoo2'}}}}};
  const github = {rest: {pulls: {get: async () => ({data: {state, head: {sha: currentSha}}}), listFiles: 'listFiles'}}, paginate: async (method, params) => {
    assert.equal(method, 'listFiles'); assert.equal(params.per_page, 100); requestedPages = true; return files;
  }};
  return {github, context, core, outputs, paginated: () => requestedPages};
}
test('PR plan uses head SHA, paginated whole-PR diff, and preview fallback', async () => {
  const fixture = planFixture({files: [{filename: 'backend/src/main.rs'}]});
  await plan(fixture);
  assert.equal(fixture.paginated(), true);
  assert.equal(fixture.outputs.sha, sha(1));
  assert.deepEqual(JSON.parse(fixture.outputs.tags), {frontend: 'preview', backend: `pr-7-${sha(1)}`, migrate: 'latest'});
  assert.deepEqual(JSON.parse(fixture.outputs.matrix), {include: [{component: 'backend'}]});
});
test('renames include the original path; huge diffs build all images', async () => {
  const rename = planFixture({files: [{filename: 'docs/old.rs', previous_filename: 'backend/src/old.rs'}]});
  await plan(rename);
  assert.deepEqual(JSON.parse(rename.outputs.matrix).include, [{component: 'backend'}]);
  const huge = planFixture({files: Array.from({length: 3000}, () => ({filename: 'docs/a.md'}))});
  await plan(huge);
  assert.equal(JSON.parse(huge.outputs.matrix).include.length, 3);
});
test('fork, closed and stale PRs do not publish', async () => {
  for (const options of [{fork: true}, {state: 'closed'}, {currentSha: sha(2)}]) {
    const fixture = planFixture(options);
    await plan(fixture);
    assert.equal(fixture.outputs.enabled, 'false');
    assert.equal(fixture.outputs.matrix, undefined);
  }
});
test('docs-only PR produces fallbacks without an empty build matrix job', async () => {
  const fixture = planFixture({files: [{filename: 'README.md'}]});
  await plan(fixture);
  assert.equal(fixture.outputs.has_images, 'false');
});
test('main plans all images with full SHA tags', async () => {
  const fixture = planFixture(); fixture.context.payload = {};
  await plan(fixture);
  assert.equal(JSON.parse(fixture.outputs.matrix).include.length, 3);
  assert.deepEqual(Object.values(JSON.parse(fixture.outputs.tags)), [sha(99), sha(99), sha(99)]);
});

import cleanup from './image-cleanup.mjs';
function cleanupFixture({dryRun = false, changedTags = false, denied = false, reopened = false} = {}) {
  const versions = [
    version(1, [`pr-7-${sha(1)}`]), version(2, []),
    version(3, ['latest']), version(4, []), version(5, [], 0.1),
  ];
  const deleted = [];
  const summary = {addHeading() {return this;}, addTable() {return this;}, async write() {}};
  let openCalls = 0;
  const github = {
    rest: {pulls: {list: 'pulls'}, packages: {
      getAllPackageVersionsForPackageOwnedByOrg: 'versions',
      getPackageVersionForPackageOwnedByOrg: async ({package_version_id}) => {
        if (denied) throw new Error('403 forbidden');
        const data = structuredClone(versions.find(v => v.id === package_version_id));
        if (changedTags) data.metadata.container.tags.push('latest');
        return {data};
      },
      deletePackageVersionForOrg: async ({package_version_id}) => {deleted.push(package_version_id);},
    }},
    paginate: async (method, params) => {
      assert.equal(params.per_page, 100);
      if (method === 'pulls') return reopened && openCalls++ > 0 ? [{number: 7}] : [];
      if (params.package_name !== 'zoo2-frontend') throw {status: 404};
      return versions;
    },
  };
  const context = {repo: {owner: 'Zli-UoA', repo: 'zoo2'}, eventName: 'schedule'};
  const core = {notice() {}, info() {}, summary};
  const fetchMock = async url => {
    if (url.includes('/token?')) return {ok: true, json: async () => ({token: 'test'})};
    const digest = url.split('/').at(-1);
    const child = digest === versions[2].name ? versions[3].name : null;
    return {ok: true, json: async () => child ? {manifests: [{digest: child}]} : {}};
  };
  return {github, context, core, deleted, fetchMock, dryRun};
}
async function runCleanupFixture(options) {
  const fixture = cleanupFixture(options);
  const originalFetch = globalThis.fetch;
  const originalDryRun = process.env.DRY_RUN;
  globalThis.fetch = fixture.fetchMock;
  process.env.DRY_RUN = String(fixture.dryRun);
  try { await cleanup(fixture); } finally {
    globalThis.fetch = originalFetch;
    if (originalDryRun === undefined) delete process.env.DRY_RUN;
    else process.env.DRY_RUN = originalDryRun;
  }
  return fixture;
}
test('cleanup API flow deletes disposable versions and protects retained index children', async () => {
  assert.deepEqual((await runCleanupFixture({})).deleted, [1, 2]);
});
test('dry run performs no deletions', async () => {
  assert.deepEqual((await runCleanupFixture({dryRun: true})).deleted, []);
});
test('tag changes, reopening, and API permission failures abort cleanup', async () => {
  await assert.rejects(runCleanupFixture({changedTags: true}), /Tags changed/);
  await assert.rejects(runCleanupFixture({reopened: true}), /PR state changed/);
  await assert.rejects(runCleanupFixture({denied: true}), /403/);
});
