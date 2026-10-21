import {execFileSync} from 'node:child_process';
import {mkdirSync, writeFileSync} from 'node:fs';
import {resolve} from 'node:path';
import {randomBytes} from 'node:crypto';
import {changedComponents, components} from './image-plan.mjs';

export const managedLabel = 'zoo2.zli.dev/preview';
const ownerLabel = 'zoo2.zli.dev/repository';
const marker = '<!-- zoo2-preview -->';
export function previewIdentity(number, domain) {
  if (!Number.isSafeInteger(number) || number < 1) throw new Error('Invalid PR number');
  const namespace = `zoo2-pr-${number}`;
  if (!domain || domain.length > 200 || !domain.split('.').every(p => /^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?$/.test(p))) throw new Error('Invalid PREVIEW_DOMAIN');
  return {number, namespace, host: `${namespace}.${domain}`};
}
export function imageTags(pr, files) {
  if (!/^[a-f0-9]{40}$/.test(pr.head.sha)) throw new Error('Invalid head SHA');
  const changed = files.length >= 3000 ? components : changedComponents(files.flatMap(f => [f.filename, f.previous_filename].filter(Boolean)));
  return Object.fromEntries(components.map(c => [c, changed.includes(c) ? `pr-${pr.number}-${pr.head.sha}` : c === 'frontend' ? 'preview' : 'latest']));
}
export function currentPR(pr, repository, sha) {
  return pr.state === 'open' && pr.head.repo?.full_name === repository && (!sha || pr.head.sha === sha);
}

// This module is loaded only from the default branch, never from a PR checkout.
export async function plan({github, context, core}) {
  const run = context.payload.workflow_run;
  if (run?.event !== 'pull_request' || run.conclusion !== 'success' || run.head_repository?.full_name !== `${context.repo.owner}/${context.repo.repo}`) return;
  const prs = await github.paginate(github.rest.pulls.list, {...context.repo, state: 'open', per_page: 100});
  const candidates = prs.filter(p => currentPR(p, `${context.repo.owner}/${context.repo.repo}`, run.head_sha)
    && (!run.pull_requests?.length || run.pull_requests.some(associated => associated.number === p.number)));
  // Empty workflow_run.pull_requests is possible. Never guess among multiple PRs.
  const pr = candidates.length === 1 ? candidates[0] : null;
  if (!pr) return;
  // Match the actual publishing workflow as well as its display name.
  const {data: workflow} = await github.rest.actions.getWorkflow({...context.repo, workflow_id: run.workflow_id});
  if (workflow.path !== '.github/workflows/cd-images.yaml') throw new Error('Unexpected image workflow');
  const identity = previewIdentity(pr.number, process.env.PREVIEW_DOMAIN);
  const files = await github.paginate(github.rest.pulls.listFiles, {...context.repo, pull_number: pr.number, per_page: 100});
  core.setOutput('plan', JSON.stringify({...identity, sha: pr.head.sha, tags: imageTags(pr, files)}));
}

const kubectl = (args, input) => execFileSync('kubectl', args, {input, encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe']});
const json = args => {
  const output = kubectl([...args, '-o', 'json']);
  return output.trim() ? JSON.parse(output) : null;
};
const apply = document => kubectl(['apply', '-f', '-'], JSON.stringify(document));
export const isManaged = (ns, repository) => /^zoo2-pr-[1-9][0-9]*$/.test(ns.metadata.name) && ns.metadata.labels?.[managedLabel] === 'true' && ns.metadata.labels?.[ownerLabel] === repository;

export function render(stage, config, infra = resolve('preview-infra')) {
  if (!['database', 'migrate', 'app'].includes(stage)) throw new Error('Invalid stage');
  const path = resolve(infra, '.preview', stage);
  mkdirSync(path, {recursive: true});
  const kustomization = {
    apiVersion: 'kustomize.config.k8s.io/v1beta1', kind: 'Kustomization', namespace: config.namespace,
    resources: [`../../kubernetes/apps/zoo2/overlays/preview/${stage}`],
    images: components.map(c => ({name: `ghcr.io/zli-uoa/zoo2-${c}`, newTag: config.tags[c]})),
  };
  if (stage === 'app') kustomization.patches = [{target: {kind: 'IngressRoute', name: 'zoo2'}, patch: JSON.stringify([
    {op: 'replace', path: '/spec/routes/0/match', value: `Host(\`${config.host}\`) && (Path(\`/api\`) || PathPrefix(\`/api/\`))`},
    {op: 'replace', path: '/spec/routes/1/match', value: `Host(\`${config.host}\`) && PathPrefix(\`/\`)`},
  ])}];
  writeFileSync(resolve(path, 'kustomization.yaml'), JSON.stringify(kustomization));
  return kubectl(['kustomize', path]);
}

async function comment(github, context, number, message) {
  const comments = await github.paginate(github.rest.issues.listComments, {...context.repo, issue_number: number, per_page: 100});
  const existing = comments.find(c => c.user?.type === 'Bot' && c.body?.startsWith(marker));
  const body = `${marker}\n${message}`;
  if (existing) await github.rest.issues.updateComment({...context.repo, comment_id: existing.id, body});
  else await github.rest.issues.createComment({...context.repo, issue_number: number, body});
}

export async function deploy({github, context, core}) {
  const config = JSON.parse(process.env.PREVIEW_PLAN);
  const {number, namespace} = previewIdentity(config.number, process.env.PREVIEW_DOMAIN);
  const repository = `${context.repo.owner}/${context.repo.repo}`;
  const getPR = async () => (await github.rest.pulls.get({...context.repo, pull_number: number})).data;
  if (!currentPR(await getPR(), repository, config.sha)) { core.notice('PR closed or changed; skipping'); return; }
  const namespaces = json(['get', 'namespaces', '-l', `${managedLabel}=true`]).items;
  const existing = json(['get', 'namespace', namespace, '--ignore-not-found']);
  if (existing?.metadata && !isManaged(existing, repository.replace('/', '.'))) throw new Error('Namespace is not owned by this preview workflow');
  if (!existing?.metadata && namespaces.length >= 3) throw new Error('Preview limit (3) reached; close an unused PR or wait for TTL cleanup');
  apply({apiVersion: 'v1', kind: 'Namespace', metadata: {name: namespace, labels: {
    [managedLabel]: 'true', [ownerLabel]: repository.replace('/', '.'),
    'pod-security.kubernetes.io/enforce': 'restricted',
  }, annotations: {'zoo2.zli.dev/updated-at': new Date().toISOString(), 'zoo2.zli.dev/head-sha': config.sha}}});
  // Create only once: a redeploy must not rotate credentials under a live DB.
  const secrets = json(['get', 'secrets', '-n', namespace]).items;
  for (const [name, stringData] of [
    ['zoo2-db-app', {username: 'zoo2', password: randomBytes(24).toString('hex')}],
    ['zoo2-electric', {secret: randomBytes(32).toString('hex')}],
  ]) if (!secrets.some(s => s.metadata.name === name)) apply({apiVersion: 'v1', kind: 'Secret', metadata: {name, namespace}, stringData});
  const dockerConfig = JSON.parse(process.env.GHCR_DOCKERCONFIGJSON);
  if (!dockerConfig.auths?.['ghcr.io']) throw new Error('GHCR_DOCKERCONFIGJSON must contain ghcr.io credentials');
  apply({apiVersion: 'v1', kind: 'Secret', type: 'kubernetes.io/dockerconfigjson', metadata: {name: 'ghcr-pull', namespace}, stringData: {'.dockerconfigjson': JSON.stringify(dockerConfig)}});
  kubectl(['apply', '-f', '-'], render('database', config));
  kubectl(['wait', '-n', namespace, '--for=condition=Ready', 'pod/postgres', '--timeout=300s']);
  // Migrations run with all previous clients stopped, and before any new client.
  if (json(['get', 'deployments', '-n', namespace]).items.length) {
    kubectl(['scale', 'deployment', '--all', '-n', namespace, '--replicas=0']);
    kubectl(['wait', 'pod', '-n', namespace, '-l', 'app.kubernetes.io/name in (frontend,backend,electric)', '--for=delete', '--timeout=180s']);
  }
  kubectl(['delete', 'job', 'zoo2-migrate', '-n', namespace, '--ignore-not-found', '--wait=true']);
  kubectl(['apply', '-f', '-'], render('migrate', config));
  kubectl(['wait', '-n', namespace, '--for=condition=complete', 'job/zoo2-migrate', '--timeout=600s']);
  const seed = json(['get', 'configmap', 'preview-seed', '-n', namespace]).data['seed.sql'];
  kubectl(['exec', '-i', '-n', namespace, 'postgres', '--', 'psql', '-v', 'ON_ERROR_STOP=1', '-U', 'zoo2', '-d', 'zoo2'], seed);
  if (!currentPR(await getPR(), repository, config.sha)) { core.notice('PR changed during migration; next run will deploy it'); return; }
  kubectl(['apply', '-f', '-'], render('app', config));
  // Applying an identical manifest does not undo the imperative scale above.
  kubectl(['scale', 'deployment', '--all', '-n', namespace, '--replicas=1']);
  for (const name of ['electric', 'backend', 'frontend']) kubectl(['rollout', 'status', `deployment/${name}`, '-n', namespace, '--timeout=300s']);
  if (currentPR(await getPR(), repository, config.sha)) await comment(github, context, number, `Preview: https://${config.host}\n\nCommit: \`${config.sha}\`\n\n開発用ユーザー切替が有効です。DB は一時データです。更新から7日後に自動回収します。`);
}

export function expired(ns, now = Date.now()) {
  const updated = Date.parse(ns.metadata.annotations?.['zoo2.zli.dev/updated-at']);
  return Number.isFinite(updated) && now - updated >= 7 * 86400000;
}
export async function cleanup({github, context}) {
  const owner = `${context.repo.owner}.${context.repo.repo}`;
  const namespaces = json(['get', 'namespaces', '-l', `${managedLabel}=true`]).items;
  for (const ns of namespaces.filter(n => isManaged(n, owner))) {
    const number = Number(ns.metadata.name.slice('zoo2-pr-'.length));
    if (context.eventName === 'pull_request_target' && number !== context.payload.pull_request.number) continue;
    const {data: pr} = await github.rest.pulls.get({...context.repo, pull_number: number});
    if (pr.state === 'open' && (context.eventName === 'pull_request_target' || !expired(ns))) continue;
    kubectl(['delete', 'namespace', ns.metadata.name, '--wait=true', '--timeout=180s']);
    await comment(github, context, number, `Preview 環境を削除しました（${pr.state === 'closed' ? 'PR 終了' : '更新から7日経過'}）。再作成するには PR を更新し、Publish images を成功させてください。`);
  }
}
