import {test} from 'node:test';
import assert from 'node:assert/strict';
import {previewIdentity, imageTags, currentPR, isManaged, managedLabel, expired, plan, promotedMainSha} from './preview.mjs';
const sha = 'a'.repeat(40);
const mainSha = 'c'.repeat(40);
const pr = {number: 74, state: 'open', head: {sha, repo: {full_name: 'Zli-UoA/zoo2'}}};
test('preview identity rejects invalid namespaces and routing injection', () => {
  assert.deepEqual(previewIdentity(74, 'example.com'), {number: 74, namespace: 'zoo2-pr-74', host: 'zoo2-pr-74.example.com'});
  for (const n of [0, -1, 1.5, '74']) assert.throws(() => previewIdentity(n, 'example.com'));
  for (const d of ['', '*.example.com', 'preview.example.com`)', '-example.com', 'EXAMPLE.com', 'a..com']) assert.throws(() => previewIdentity(74, d));
});
test('unchanged components pin one main commit; renamed and truncated diffs select builds', () => {
  assert.deepEqual(imageTags(pr, [], mainSha), {frontend: `preview-${mainSha}`, backend: mainSha, migrate: mainSha});
  assert.deepEqual(imageTags(pr, [{filename: 'docs/old.rs', previous_filename: 'backend/old.rs'}], mainSha), {frontend: `preview-${mainSha}`, backend: `pr-74-${sha}`, migrate: mainSha});
  assert.deepEqual(Object.values(imageTags(pr, Array.from({length: 3000}, () => ({filename: 'README.md'})))), Array(3).fill(`pr-74-${sha}`));
  for (const invalid of [undefined, 'latest', 'preview']) assert.throws(() => imageTags(pr, [], invalid));
});
test('the promoted main commit comes from the latest alias, not the newest version', async () => {
  const github = {rest: {packages: {getAllPackageVersionsForPackageOwnedByOrg: 'versions'}}, paginate: async (_, params) => {
    assert.deepEqual(params, {org: 'Zli-UoA', package_type: 'container', package_name: 'zoo2-backend', per_page: 100});
    return [{metadata: {container: {tags: ['d'.repeat(40)]}}}, {metadata: {container: {tags: ['latest', mainSha]}}}];
  }};
  assert.equal(await promotedMainSha(github, 'Zli-UoA'), mainSha);
  await assert.rejects(promotedMainSha({...github, paginate: async () => [{metadata: {container: {tags: ['latest']}}}]}, 'Zli-UoA'));
});
test('only an open same-repository current PR can deploy', () => {
  assert.ok(currentPR(pr, 'Zli-UoA/zoo2', sha));
  assert.ok(!currentPR({...pr, state: 'closed'}, 'Zli-UoA/zoo2', sha));
  assert.ok(!currentPR(pr, 'fork/zoo2', sha));
  assert.ok(!currentPR(pr, 'Zli-UoA/zoo2', 'b'.repeat(40)));
});
test('cleanup requires exact namespace ownership and a finite TTL', () => {
  const ns = {metadata: {name: 'zoo2-pr-74', labels: {[managedLabel]: 'true', 'zoo2.zli.dev/repository': 'Zli-UoA.zoo2'}, annotations: {'zoo2.zli.dev/updated-at': '2026-10-01T00:00:00Z'}}};
  assert.ok(isManaged(ns, 'Zli-UoA.zoo2'));
  assert.ok(!isManaged({...ns, metadata: {...ns.metadata, name: 'zoo2'}}, 'Zli-UoA.zoo2'));
  assert.ok(!isManaged(ns, 'another.repo'));
  assert.ok(!expired(ns, Date.parse('2026-10-07T23:59:59Z')));
  assert.ok(expired(ns, Date.parse('2026-10-08T00:00:00Z')));
  assert.ok(!expired({metadata: {}}));
});
test('only successful PR publishing runs for the current SHA produce a plan', async () => {
  const originalDomain = process.env.PREVIEW_DOMAIN;
  process.env.PREVIEW_DOMAIN = 'example.com';
  try {
    for (const mutation of [{}, {conclusion: 'failure'}, {event: 'push'}, {head_sha: 'b'.repeat(40)}, {head_repository: {full_name: 'fork/zoo2'}}]) {
      const output = {};
      const context = {repo: {owner: 'Zli-UoA', repo: 'zoo2'}, payload: {workflow_run: {event: 'pull_request', conclusion: 'success', head_sha: sha, head_repository: {full_name: 'Zli-UoA/zoo2'}, workflow_id: 1, ...mutation}}};
      const versions = [{metadata: {container: {tags: ['latest', mainSha]}}}];
      const github = {rest: {pulls: {list: 'prs', listFiles: 'files'}, packages: {getAllPackageVersionsForPackageOwnedByOrg: 'versions'}, actions: {getWorkflow: async () => ({data: {path: '.github/workflows/cd-images.yaml'}})}}, paginate: async method => method === 'prs' ? [pr] : method === 'versions' ? versions : []};
      await plan({github, context, core: {setOutput: (key, value) => {output[key] = value;}}});
      if (!Object.keys(mutation).length) assert.deepEqual(JSON.parse(output.plan).tags, {frontend: `preview-${mainSha}`, backend: mainSha, migrate: mainSha});
      else assert.equal(output.plan, undefined);
    }
  } finally {
    if (originalDomain === undefined) delete process.env.PREVIEW_DOMAIN;
    else process.env.PREVIEW_DOMAIN = originalDomain;
  }
});
