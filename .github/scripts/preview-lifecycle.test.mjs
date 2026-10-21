import {test} from 'node:test';
import assert from 'node:assert/strict';
import {mkdtempSync, writeFileSync, readFileSync, mkdirSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {resolve} from 'node:path';
import {deploy, cleanup, managedLabel} from './preview.mjs';

const sha = 'a'.repeat(40);
const repository = 'Zli-UoA/zoo2';
const pr = {number: 74, state: 'open', head: {sha, repo: {full_name: repository}}};
const context = {repo: {owner: 'Zli-UoA', repo: 'zoo2'}, eventName: 'schedule', payload: {}};
async function fixture(options, callback) {
  const directory = mkdtempSync(resolve(tmpdir(), 'preview-test-'));
  const previous = {...process.env};
  const cwd = process.cwd();
  const log = resolve(directory, 'calls.jsonl');
  mkdirSync(resolve(directory, 'bin'));
  writeFileSync(resolve(directory, 'bin/kubectl'), `#!/usr/bin/env node
import fs from 'node:fs';
const args = process.argv.slice(2);
const input = fs.readFileSync(0, 'utf8');
fs.appendFileSync(process.env.MOCK_LOG, JSON.stringify({args, input}) + '\\n');
const options = JSON.parse(process.env.MOCK_OPTIONS);
if (args[0] === 'wait' && args.includes('job/zoo2-migrate') && options.migrateFails) process.exit(1);
if (args[0] === 'wait' && args.includes('--for=delete') && !options.pods) { process.stderr.write('error: no matching resources found'); process.exit(1); }
if (args[0] === 'delete' && args[1] === 'namespace' && (options.deleteFails ?? []).includes(args[2])) { process.stderr.write('error: timed out'); process.exit(1); }
if (args[0] === 'get') {
  const items = {namespaces: options.namespaces, deployments: options.deployments && [{}], pods: options.pods && [{}]}[args[1]];
  if (args[1] === 'namespace') process.stdout.write('');
  else if (args[1] === 'configmap') process.stdout.write(JSON.stringify({data: {'seed.sql': 'SELECT 1;'}}));
  else process.stdout.write(JSON.stringify({items: items ?? []}));
} else if (args[0] === 'kustomize') process.stdout.write(JSON.stringify({stage: args[1].split('/').at(-1)}));
`, {mode: 0o755});
  process.env.PATH = `${directory}/bin:${process.env.PATH}`;
  process.env.MOCK_LOG = log;
  process.env.MOCK_OPTIONS = JSON.stringify(options);
  process.env.PREVIEW_PLAN = JSON.stringify({number: 74, namespace: 'zoo2-pr-74', host: 'zoo2-pr-74.example.com', sha, tags: {frontend: `preview-${'c'.repeat(40)}`, backend: 'c'.repeat(40), migrate: 'c'.repeat(40)}});
  process.env.PREVIEW_DOMAIN = 'example.com';
  process.env.GHCR_DOCKERCONFIGJSON = JSON.stringify({auths: {'ghcr.io': {auth: 'test'}}});
  let reads = 0;
  const comments = [];
  const errors = [];
  const github = {rest: {
    pulls: {get: async ({pull_number}) => ({data: pull_number !== 74 ? {...pr, number: pull_number, state: 'closed'} : (++reads > 1 && options.staleAfterMigration) ? {...pr, head: {...pr.head, sha: 'b'.repeat(40)}} : {...pr, state: options.state ?? 'open'}})},
    issues: {listComments: 'comments', createComment: async c => comments.push(c), updateComment: async c => comments.push(c)},
  }, paginate: async () => []};
  process.chdir(directory);
  try {
    await callback({github, context, core: {notice() {}, error: e => errors.push(e)}, comments, errors, calls: () => {
      try { return readFileSync(log, 'utf8').trim().split('\n').filter(Boolean).map(JSON.parse); }
      catch { return []; }
    }});
  } finally {
    process.chdir(cwd);
    for (const key of Object.keys(process.env)) if (!(key in previous)) delete process.env[key];
    Object.assign(process.env, previous);
    rmSync(directory, {recursive: true, force: true});
  }
}
test('deploy waits for migration and seeding before apps and posts one URL', async () => {
  await fixture({}, async f => {
    await deploy(f);
    const calls = f.calls();
    const migration = calls.findIndex(c => c.args.includes('job/zoo2-migrate') && c.args[0] === 'wait');
    const seed = calls.findIndex(c => c.args[0] === 'exec');
    const app = calls.findIndex(c => c.args[0] === 'apply' && c.input === '{"stage":"app"}');
    assert.ok(migration >= 0 && seed > migration && app > seed);
    assert.equal(f.comments.length, 1);
    assert.match(f.comments[0].body, /https:\/\/zoo2-pr-74.example.com/);
  });
});
test('migration failure and a head change never start new apps or post readiness', async () => {
  for (const options of [{migrateFails: true}, {staleAfterMigration: true}]) {
    await fixture(options, async f => {
      if (options.migrateFails) await assert.rejects(deploy(f));
      else await deploy(f);
      assert.ok(!f.calls().some(c => c.input === '{"stage":"app"}'));
      assert.equal(f.comments.length, 0);
    });
  }
});
test('a closed PR cannot mutate the cluster', async () => {
  await fixture({state: 'closed'}, async f => {
    await deploy(f);
    assert.deepEqual(f.calls(), []);
  });
});
test('cleanup deletes only owned closed previews and preserves reopened PRs', async () => {
  const ns = {metadata: {name: 'zoo2-pr-74', labels: {[managedLabel]: 'true', 'zoo2.zli.dev/repository': 'Zli-UoA.zoo2'}}};
  for (const state of ['closed', 'open']) {
    await fixture({state, namespaces: [ns, {metadata: {...ns.metadata, name: 'zoo2'}}, {metadata: {...ns.metadata, name: 'zoo2-pr-75', labels: {[managedLabel]: 'true'}}}]}, async f => {
      await cleanup(f);
      assert.deepEqual(f.calls().filter(c => c.args[0] === 'delete').map(c => c.args[2]), state === 'closed' ? ['zoo2-pr-74'] : []);
    });
  }
});
test('redeploy stops previous clients, waiting only for Pods that still exist', async () => {
  for (const pods of [true, false]) {
    await fixture({deployments: true, pods}, async f => {
      await deploy(f);
      const calls = f.calls();
      const wait = calls.findIndex(c => c.args[0] === 'wait' && c.args.includes('--for=delete'));
      assert.ok(calls.some(c => c.args[0] === 'scale' && c.args.includes('--replicas=0')));
      assert.equal(wait >= 0, pods);
      assert.equal(f.comments.length, 1);
    });
  }
});
test('one stuck Namespace does not stop reclaiming the others', async () => {
  const labels = {[managedLabel]: 'true', 'zoo2.zli.dev/repository': 'Zli-UoA.zoo2'};
  const namespaces = [74, 75].map(n => ({metadata: {name: `zoo2-pr-${n}`, labels}}));
  await fixture({state: 'closed', namespaces, deleteFails: ['zoo2-pr-74']}, async f => {
    await assert.rejects(cleanup(f), /zoo2-pr-74/);
    assert.deepEqual(f.calls().filter(c => c.args[0] === 'delete').map(c => c.args[2]), ['zoo2-pr-74', 'zoo2-pr-75']);
    assert.deepEqual(f.comments.map(c => c.issue_number), [75]);
    assert.match(f.errors[0], /zoo2-pr-74: error: timed out/);
  });
});
