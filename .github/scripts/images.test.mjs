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
  assert.deepEqual(candidates(versions, new Map()).map(v => v.id), [16, 17]);
  assert.deepEqual(candidates(versions.map(v => version(v.id, [`preview-${sha(v.id)}`])), new Map()).map(v => v.id), [16, 17]);
  // main を発行済みの古いコミットに戻した場合は、15世代より古くてもそのイメージを残す。
  assert.deepEqual(candidates(versions, new Map(), {mainHead: sha(17)}).map(v => v.id), [16]);
  assert.deepEqual(candidates(versions.map(v => version(v.id, [`preview-${sha(v.id)}`])), new Map(), {mainHead: sha(16)}).map(v => v.id), [17]);
  assert.deepEqual(candidates([1, 2, 3].map(i => version(i, [`pr-7-${sha(i)}`])), new Map([[7, sha(99)]])).map(v => v.id), [3]);
  // 古い SHA へ force-push で戻した場合は、2世代より古くても head のイメージを残す。
  assert.deepEqual(candidates([1, 2, 3].map(i => version(i, [`pr-7-${sha(i)}`])), new Map([[7, sha(3)]])), []);
});
test('closed PR, exact PR boundary, protected and unknown tags', () => {
  const versions = [version(1, [`pr-7-${sha(1)}`]), version(2, [`pr-70-${sha(2)}`]), version(3, [`pr-7-${sha(3)}`, 'latest']), version(4, ['custom']), version(5, ['preview'])];
  assert.deepEqual(candidates(versions, new Map(), {closedPR: 7}).map(v => v.id), [1]);
  assert.deepEqual(candidates(versions, new Map([[7, sha(99)]]), {closedPR: 7}), []);
  assert.deepEqual(candidates(versions, new Map()).map(v => v.id), [1, 2]);
});
test('multiple tags preserve version if any tag is retained', () => {
  const versions = Array.from({length: 17}, (_, i) => version(i + 1, [sha(i + 1)]));
  versions[16].metadata.container.tags.push(`pr-9-${sha(17)}`);
  assert.deepEqual(candidates(versions, new Map([[9, sha(99)]])).map(v => v.id), [16]);
});
test('a PR alias on a main version never deletes the main image', () => {
  // 変更のないコンポーネントの pr タグは、main の `<SHA>` と同じバージョンに付く。
  const aliased = [version(1, [sha(1), `pr-7-${sha(50)}`]), version(2, [`preview-${sha(1)}`, `pr-7-${sha(50)}`])];
  assert.deepEqual(candidates(aliased, new Map(), {closedPR: 7}), []);
  assert.deepEqual(candidates(aliased, new Map()), []);
  // main の15世代より古くても、open な PR の head が使っている間は残す。
  const old = [...Array.from({length: 15}, (_, i) => version(i + 1, [sha(i + 1)])), version(16, [sha(16), `pr-7-${sha(50)}`])];
  assert.deepEqual(candidates(old, new Map([[7, sha(50)]])), []);
});
test('untagged grace period', () => {
  assert.deepEqual(candidates([version(1, [], 0.5), version(2, [], 2)], new Map()).map(v => v.id), [2]);
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

import plan, {promotedMainSha} from './image-plan.mjs';
function planFixture({files = [], fork = false, state = 'open', currentSha = sha(1)} = {}) {
  const outputs = {};
  let requestedPages = false;
  const core = {setOutput: (key, value) => {outputs[key] = value;}, summary: {addHeading() {return this;}, addCodeBlock() {return this;}, async write() {}}};
  const context = {repo: {owner: 'Zli-UoA', repo: 'zoo2'}, sha: sha(99), payload: {pull_request: {number: 7, head: {sha: sha(1), repo: {full_name: fork ? 'someone/zoo2' : 'Zli-UoA/zoo2'}}}}};
  let requestedVersions = false;
  const github = {rest: {pulls: {get: async () => ({data: {state, head: {sha: currentSha}}}), listFiles: 'listFiles'}, packages: {getAllPackageVersionsForPackageOwnedByOrg: 'versions'}}, paginate: async (method, params) => {
    assert.equal(params.per_page, 100);
    // latest は main の SHA 98 を指す。より新しいバージョンがあっても昇格済みの SHA を使う。
    if (method === 'versions') { requestedVersions = true; return [version(2, [sha(97)]), version(1, ['latest', sha(98)])]; }
    assert.equal(method, 'listFiles'); requestedPages = true; return files;
  }};
  return {github, context, core, outputs, paginated: () => requestedPages, readVersions: () => requestedVersions};
}
test('PR plan uses head SHA, paginated whole-PR diff, and aliases unchanged components to the promoted main', async () => {
  const fixture = planFixture({files: [{filename: 'backend/src/main.rs'}]});
  await plan(fixture);
  assert.equal(fixture.paginated(), true);
  assert.equal(fixture.outputs.sha, sha(1));
  assert.equal(fixture.outputs.pr, '7');
  assert.deepEqual(JSON.parse(fixture.outputs.tags), {frontend: `pr-7-${sha(1)}`, backend: `pr-7-${sha(1)}`, migrate: `pr-7-${sha(1)}`});
  assert.deepEqual(JSON.parse(fixture.outputs.aliases), [{component: 'frontend', source: `preview-${sha(98)}`}, {component: 'migrate', source: sha(98)}]);
  assert.deepEqual(JSON.parse(fixture.outputs.matrix), {include: [{component: 'backend'}]});
});
test('the promoted main commit comes from the latest alias, not the newest version', async () => {
  const github = {rest: {packages: {getAllPackageVersionsForPackageOwnedByOrg: 'versions'}}, paginate: async (_, params) => {
    assert.deepEqual(params, {org: 'Zli-UoA', package_type: 'container', package_name: 'zoo2-backend', per_page: 100});
    return [version(2, [sha(97)]), version(1, ['latest', sha(98)])];
  }};
  assert.equal(await promotedMainSha(github, 'Zli-UoA'), sha(98));
  await assert.rejects(promotedMainSha({...github, paginate: async () => [version(1, ['latest'])]}, 'Zli-UoA'));
});
test('renames include the original path; huge diffs build all images', async () => {
  const rename = planFixture({files: [{filename: 'docs/old.rs', previous_filename: 'backend/src/old.rs'}]});
  await plan(rename);
  assert.deepEqual(JSON.parse(rename.outputs.matrix).include, [{component: 'backend'}]);
  const huge = planFixture({files: Array.from({length: 3000}, () => ({filename: 'docs/a.md'}))});
  await plan(huge);
  assert.equal(JSON.parse(huge.outputs.matrix).include.length, 3);
  // すべてビルドする場合は別名がなく、main の SHA も調べない。
  assert.deepEqual(JSON.parse(huge.outputs.aliases), []);
  assert.equal(huge.readVersions(), false);
});
test('fork, closed and stale PRs do not publish', async () => {
  for (const options of [{fork: true}, {state: 'closed'}, {currentSha: sha(2)}]) {
    const fixture = planFixture(options);
    await plan(fixture);
    assert.equal(fixture.outputs.enabled, 'false');
    assert.equal(fixture.outputs.matrix, undefined);
  }
});
test('docs-only PR aliases every component without an empty build matrix job', async () => {
  const fixture = planFixture({files: [{filename: 'README.md'}]});
  await plan(fixture);
  assert.equal(fixture.outputs.has_images, 'false');
  assert.deepEqual(JSON.parse(fixture.outputs.aliases).map(a => a.source), [`preview-${sha(98)}`, sha(98), sha(98)]);
});
test('main plans all images with full SHA tags', async () => {
  const fixture = planFixture(); fixture.context.payload = {};
  await plan(fixture);
  assert.equal(JSON.parse(fixture.outputs.matrix).include.length, 3);
  assert.deepEqual(Object.values(JSON.parse(fixture.outputs.tags)), [sha(99), sha(99), sha(99)]);
  assert.deepEqual(JSON.parse(fixture.outputs.aliases), []);
  assert.equal(fixture.outputs.pr, '');
});

import cleanup from './image-cleanup.mjs';
function cleanupFixture({dryRun = false, changedTags = false, denied = false, reopened = false, publishing = false, missingManifest = false, deleteFails = false} = {}) {
  const versions = [
    version(1, [`pr-7-${sha(1)}`]), version(2, []),
    version(3, ['latest']), version(4, []), version(5, [], 0.1), version(6, []),
  ];
  // index 3 は残す対象で 4 を参照する。index 1 は削除してよく、6 を参照する。
  const children = {[versions[2].name]: versions[3].name, [versions[0].name]: versions[5].name};
  const deleted = [];
  const rows = [];
  const summary = {addHeading() {return this;}, addTable(table) {rows.push(...table.slice(1)); return this;}, async write() {}};
  const calls = {pulls: 0};
  const github = {
    rest: {
      repos: {getBranch: async ({branch}) => ({data: {commit: {sha: branch === 'main' ? sha(98) : null}}})},
      pulls: {list: 'pulls', get: async ({pull_number}) => ({data: {state: reopened && pull_number === 7 ? 'open' : 'closed', head: {sha: sha(50)}}})},
      actions: {listWorkflowRuns: async ({workflow_id, status}) => {
        assert.equal(workflow_id, 'cd-images.yaml');
        return {data: {total_count: publishing && status === 'in_progress' ? 1 : 0}};
      }},
      packages: {
        getAllPackageVersionsForPackageOwnedByOrg: 'versions',
        getPackageVersionForOrganization: async ({package_version_id}) => {
          if (denied) throw new Error('403 forbidden');
          const data = structuredClone(versions.find(v => v.id === package_version_id));
          if (changedTags && package_version_id === 1) data.metadata.container.tags.push('latest');
          return {data};
        },
        deletePackageVersionForOrg: async ({package_version_id}) => {
          if (deleteFails && deleted.length) throw new Error('500 delete');
          deleted.push(package_version_id);
        },
      },
    },
    paginate: async (method, params) => {
      assert.equal(params.per_page, 100);
      if (method === 'pulls') { calls.pulls++; return []; }
      if (params.package_name !== 'zoo2-frontend') throw {status: 404};
      return versions;
    },
  };
  const context = {repo: {owner: 'Zli-UoA', repo: 'zoo2'}, eventName: 'schedule'};
  const core = {notice() {}, info() {}, summary};
  const fetchMock = async url => {
    if (url.includes('/token?')) return {ok: true, json: async () => ({token: 'test'})};
    const digest = url.split('/').at(-1);
    if (missingManifest && digest === versions[2].name) return {ok: false, status: 404};
    const child = children[digest];
    return {ok: true, status: 200, json: async () => child ? {manifests: [{digest: child}]} : {}};
  };
  return {github, context, core, deleted, rows, calls, fetchMock, dryRun};
}
async function runCleanupFixture(options, fixture = cleanupFixture(options)) {
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
  const fixture = await runCleanupFixture({});
  assert.deepEqual(fixture.deleted, [1, 2, 6]);
  assert.equal(fixture.calls.pulls, 1);
});
test('dry run performs no deletions', async () => {
  const fixture = await runCleanupFixture({dryRun: true});
  assert.deepEqual(fixture.deleted, []);
  assert.deepEqual(fixture.rows.map(r => r[1]), ['1', '2', '6']);
});
test('tag changes and reopened PRs keep the version and its children without aborting', async () => {
  assert.deepEqual((await runCleanupFixture({changedTags: true})).deleted, [2]);
  assert.deepEqual((await runCleanupFixture({reopened: true})).deleted, [2]);
});
test('running publication defers untagged deletion only', async () => {
  assert.deepEqual((await runCleanupFixture({publishing: true})).deleted, [1]);
});
test('a missing retained manifest protects nothing', async () => {
  assert.deepEqual((await runCleanupFixture({missingManifest: true})).deleted, [1, 2, 4, 6]);
});
test('API failures abort cleanup but still record completed deletions', async () => {
  await assert.rejects(runCleanupFixture({denied: true}), /403/);
  const fixture = cleanupFixture({deleteFails: true});
  await assert.rejects(runCleanupFixture({}, fixture), /500/);
  assert.deepEqual(fixture.rows.map(r => r[1]), ['1']);
});
test('manifest reads stay within the concurrency limit across nested indexes', async () => {
  const manifests = {root: {manifests: Array.from({length: 20}, (_, i) => ({digest: `child${i}`}))}};
  let active = 0, peak = 0;
  const read = async digest => {
    peak = Math.max(peak, ++active);
    await new Promise(resolve => setTimeout(resolve, 1));
    active--;
    return manifests[digest] ?? {};
  };
  assert.equal((await referencedDigests(['root'], read, {concurrency: 3})).size, 21);
  assert.equal(peak, 3);
});
