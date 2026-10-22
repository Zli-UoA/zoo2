// PR ごとの Preview 環境を作成・回収するスクリプト。cd-preview.yaml から次の3つを呼ぶ。
// - plan:    Publish images の完了を受け、デプロイ対象の PR と使うイメージを決める（クラスタには触れない）
// - deploy:  plan の結果をもとに site1 の Namespace へ DB・マイグレーション・アプリを順に展開する
// - cleanup: close 済みまたは TTL 切れの Preview の Namespace を削除する
import {execFileSync} from 'node:child_process';
import {mkdirSync, writeFileSync} from 'node:fs';
import {resolve} from 'node:path';
import {randomBytes} from 'node:crypto';
import {changedComponents, components} from './image-plan.mjs';

// このワークフローが作った Namespace に付けるラベル。回収時はこのラベルと所有リポジトリの両方を確認する。
export const managedLabel = 'zoo2.zli.dev/preview';
const ownerLabel = 'zoo2.zli.dev/repository';
// PR コメントを作り直さず更新するための目印。
const marker = '<!-- zoo2-preview -->';

// PR 番号から Namespace 名とホスト名を決める。
// 値は Kubernetes のリソース名と Traefik のルール式に埋め込むため、不正な番号やドメインはここで弾く。
export function previewIdentity(number, domain) {
  if (!Number.isSafeInteger(number) || number < 1) throw new Error('Invalid PR number');
  const namespace = `zoo2-pr-${number}`;
  if (!domain || domain.length > 200 || !domain.split('.').every(p => /^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?$/.test(p))) throw new Error('Invalid PREVIEW_DOMAIN');
  return {number, namespace, host: `${namespace}.${domain}`};
}

// PR で変更されたコンポーネントを返す。リネーム元のパスも変更として数える。
// GitHub の変更ファイル一覧は3000件で打ち切られるため、3000件以上なら全コンポーネントを変更ありとみなす。
export function changedForPR(files) {
  return files.length >= 3000 ? components : changedComponents(files.flatMap(f => [f.filename, f.previous_filename].filter(Boolean)));
}

// コンポーネントごとに使うイメージタグを決める。
// - PR で変更あり: その PR 用にビルドした `pr-<番号>-<head SHA>`
// - 変更なし:     昇格済み main コミットの不変タグ（backend・migrate は `<SHA>`、frontend は Preview 用ビルドの `preview-<SHA>`）
// 可変タグの latest・preview は使わない。Pod の再作成や後続の main 昇格で、
// ここでマイグレーションしたスキーマと合わない新しい backend が起動するのを防ぐため。
export function imageTags(pr, files, mainSha) {
  if (!/^[a-f0-9]{40}$/.test(pr.head.sha)) throw new Error('Invalid head SHA');
  const changed = changedForPR(files);
  if (changed.length < components.length && !/^[a-f0-9]{40}$/.test(mainSha ?? '')) throw new Error('Invalid main SHA');
  return Object.fromEntries(components.map(c => [c, changed.includes(c) ? `pr-${pr.number}-${pr.head.sha}` : c === 'frontend' ? `preview-${mainSha}` : mainSha]));
}

// 現在 latest が指している main コミットの SHA を GHCR から取得する。
// latest は `<SHA>` タグと同じバージョンを指すため、そのバージョンに付いている40桁のタグを読む。
// 昇格は全コンポーネントのビルド成功後に行われるので、backend だけ見れば他のコンポーネントにも同じ SHA のイメージがある。
export async function promotedMainSha(github, org) {
  const versions = await github.paginate(github.rest.packages.getAllPackageVersionsForPackageOwnedByOrg, {org, package_type: 'container', package_name: 'zoo2-backend', per_page: 100});
  const tags = versions.find(v => v.metadata?.container?.tags?.includes('latest'))?.metadata.container.tags ?? [];
  const sha = tags.find(t => /^[a-f0-9]{40}$/.test(t));
  if (!sha) throw new Error('zoo2-backend:latest has no main commit tag');
  return sha;
}

// open で、fork ではなく同じリポジトリのブランチの PR か。sha を渡した場合は、head がその SHA のままかも確認する。
export function currentPR(pr, repository, sha) {
  return pr.state === 'open' && pr.head.repo?.full_name === repository && (!sha || pr.head.sha === sha);
}

// デプロイ計画を作り、`plan` 出力に JSON で渡す。対象外の場合は何も出力せず、deploy ジョブも動かない。
// このモジュールは既定ブランチから読み込む。PR のコードを checkout して実行することはない。
export async function plan({github, context, core}) {
  const run = context.payload.workflow_run;
  // 同じリポジトリの PR で成功した Publish images だけを対象にする。
  if (run?.event !== 'pull_request' || run.conclusion !== 'success' || run.head_repository?.full_name !== `${context.repo.owner}/${context.repo.repo}`) return;
  // ビルドした SHA が今も head の open PR を探す。
  const prs = await github.paginate(github.rest.pulls.list, {...context.repo, state: 'open', per_page: 100});
  const candidates = prs.filter(p => currentPR(p, `${context.repo.owner}/${context.repo.repo}`, run.head_sha)
    && (!run.pull_requests?.length || run.pull_requests.some(associated => associated.number === p.number)));
  // workflow_run.pull_requests は空のことがある。候補が複数あるときは推測せずデプロイしない。
  const pr = candidates.length === 1 ? candidates[0] : null;
  if (!pr) return;
  // 表示名が同じだけの別ワークフローに反応しないよう、ファイルのパスでも確認する。
  const {data: workflow} = await github.rest.actions.getWorkflow({...context.repo, workflow_id: run.workflow_id});
  if (workflow.path !== '.github/workflows/cd-images.yaml') throw new Error('Unexpected image workflow');
  const identity = previewIdentity(pr.number, process.env.PREVIEW_DOMAIN);
  const files = await github.paginate(github.rest.pulls.listFiles, {...context.repo, pull_number: pr.number, per_page: 100});
  // 変更のないコンポーネントがあるときだけ main の SHA を調べる。
  const mainSha = changedForPR(files).length < components.length ? await promotedMainSha(github, context.repo.owner) : null;
  core.setOutput('plan', JSON.stringify({...identity, sha: pr.head.sha, tags: imageTags(pr, files, mainSha)}));
}

// kubectl のヘルパー。失敗すると execFileSync が例外を投げ、error.stderr に kubectl のエラー出力が入る。
const kubectl = (args, input) => execFileSync('kubectl', args, {input, encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe']});
// `-o json` の結果をパースする。`--ignore-not-found` で該当がなく出力が空の場合は null を返す。
const json = args => {
  const output = kubectl([...args, '-o', 'json']);
  return output.trim() ? JSON.parse(output) : null;
};
const apply = document => kubectl(['apply', '-f', '-'], JSON.stringify(document));
// このワークフローが作った Namespace か。名前の形式、管理ラベル、所有リポジトリの3つがすべて一致する場合だけ true。
export const isManaged = (ns, repository) => /^zoo2-pr-[1-9][0-9]*$/.test(ns.metadata.name) && ns.metadata.labels?.[managedLabel] === 'true' && ns.metadata.labels?.[ownerLabel] === repository;

// zoo2-infra の preview overlay を元に、指定した段階のマニフェストを生成する。
// 段階は database（PostgreSQL）、migrate（Atlas Job）、app（Electric・backend・frontend・Ingress）の3つ。
// 一時的な kustomization で Namespace とイメージタグを差し替え、app では IngressRoute のホスト名を PR のものに書き換える。
export function render(stage, config, infra = resolve('preview-infra')) {
  if (!['database', 'migrate', 'app'].includes(stage)) throw new Error('Invalid stage');
  const path = resolve(infra, '.preview', stage);
  mkdirSync(path, {recursive: true});
  const kustomization = {
    apiVersion: 'kustomize.config.k8s.io/v1beta1', kind: 'Kustomization', namespace: config.namespace,
    resources: [`../../kubernetes/apps/zoo2/overlays/preview/${stage}`],
    images: components.map(c => ({name: `ghcr.io/zli-uoa/zoo2-${c}`, newTag: config.tags[c]})),
  };
  // routes[0] は /api を backend へ、routes[1] はそれ以外を frontend へ転送する。
  if (stage === 'app') kustomization.patches = [{target: {kind: 'IngressRoute', name: 'zoo2'}, patch: JSON.stringify([
    {op: 'replace', path: '/spec/routes/0/match', value: `Host(\`${config.host}\`) && (Path(\`/api\`) || PathPrefix(\`/api/\`))`},
    {op: 'replace', path: '/spec/routes/1/match', value: `Host(\`${config.host}\`) && PathPrefix(\`/\`)`},
  ])}];
  writeFileSync(resolve(path, 'kustomization.yaml'), JSON.stringify(kustomization));
  return kubectl(['kustomize', path]);
}

// PR に Preview の状態をコメントする。Bot が以前に書いた marker 付きのコメントがあれば、新しく作らずに更新する。
async function comment(github, context, number, message) {
  const comments = await github.paginate(github.rest.issues.listComments, {...context.repo, issue_number: number, per_page: 100});
  const existing = comments.find(c => c.user?.type === 'Bot' && c.body?.startsWith(marker));
  const body = `${marker}\n${message}`;
  if (existing) await github.rest.issues.updateComment({...context.repo, comment_id: existing.id, body});
  else await github.rest.issues.createComment({...context.repo, issue_number: number, body});
}

// plan の結果（環境変数 PREVIEW_PLAN）をもとに Preview をデプロイする。手順は次のとおり。
// 1. PR の確認と Namespace の作成  2. Secret の作成  3. PostgreSQL の起動
// 4. 既存アプリの停止とマイグレーション  5. seed の投入  6. アプリの起動と PR へのコメント
export async function deploy({github, context, core}) {
  const config = JSON.parse(process.env.PREVIEW_PLAN);
  const {number, namespace} = previewIdentity(config.number, process.env.PREVIEW_DOMAIN);
  const repository = `${context.repo.owner}/${context.repo.repo}`;
  const getPR = async () => (await github.rest.pulls.get({...context.repo, pull_number: number})).data;
  // plan の後に PR が close されたり、新しいコミットが push されたりした場合はクラスタに触れずに終わる。
  if (!currentPR(await getPR(), repository, config.sha)) { core.notice('PR closed or changed; skipping'); return; }

  // 1. Namespace。同名の Namespace が他の用途で使われていれば中断する。新規作成は同時に3環境まで。
  const namespaces = json(['get', 'namespaces', '-l', `${managedLabel}=true`]).items;
  const existing = json(['get', 'namespace', namespace, '--ignore-not-found']);
  if (existing?.metadata && !isManaged(existing, repository.replace('/', '.'))) throw new Error('Namespace is not owned by this preview workflow');
  if (!existing?.metadata && namespaces.length >= 3) throw new Error('Preview limit (3) reached; close an unused PR or wait for TTL cleanup');
  // updated-at は TTL 回収の起点になる。デプロイのたびに更新する。
  apply({apiVersion: 'v1', kind: 'Namespace', metadata: {name: namespace, labels: {
    [managedLabel]: 'true', [ownerLabel]: repository.replace('/', '.'),
    'pod-security.kubernetes.io/enforce': 'restricted',
  }, annotations: {'zoo2.zli.dev/updated-at': new Date().toISOString(), 'zoo2.zli.dev/head-sha': config.sha}}});

  // 2. Secret。DB と Electric の認証情報は初回だけランダムに作る。
  // 再デプロイで作り直すと、稼働中の DB とパスワードが合わなくなるため。
  const secrets = json(['get', 'secrets', '-n', namespace]).items;
  for (const [name, stringData] of [
    ['zoo2-db-app', {username: 'zoo2', password: randomBytes(24).toString('hex')}],
    ['zoo2-electric', {secret: randomBytes(32).toString('hex')}],
  ]) if (!secrets.some(s => s.metadata.name === name)) apply({apiVersion: 'v1', kind: 'Secret', metadata: {name, namespace}, stringData});
  // GHCR の pull 用 Secret は毎回上書きする。
  const dockerConfig = JSON.parse(process.env.GHCR_DOCKERCONFIGJSON);
  if (!dockerConfig.auths?.['ghcr.io']) throw new Error('GHCR_DOCKERCONFIGJSON must contain ghcr.io credentials');
  apply({apiVersion: 'v1', kind: 'Secret', type: 'kubernetes.io/dockerconfigjson', metadata: {name: 'ghcr-pull', namespace}, stringData: {'.dockerconfigjson': JSON.stringify(dockerConfig)}});

  // 3. PostgreSQL を起動し、Ready になるまで待つ。
  kubectl(['apply', '-f', '-'], render('database', config));
  kubectl(['wait', '-n', namespace, '--for=condition=Ready', 'pod/postgres', '--timeout=300s']);

  // 4. マイグレーション。古いアプリをすべて止めてから実行し、新しいアプリはマイグレーションが終わるまで起動しない。
  if (json(['get', 'deployments', '-n', namespace]).items.length) {
    kubectl(['scale', 'deployment', '--all', '-n', namespace, '--replicas=0']);
    const clients = ['-n', namespace, '-l', 'app.kubernetes.io/name in (frontend,backend,electric)'];
    // 前回の実行が途中で止まって replica が0のままだと Pod は1つもない。
    // kubectl wait は対象が0件だとエラーになるため、Pod が残っているときだけ削除を待つ。
    if (json(['get', 'pods', ...clients]).items.length) kubectl(['wait', 'pod', ...clients, '--for=delete', '--timeout=180s']);
  }
  // Job は再実行できないため、前回の Job を消してから作り直す。完了しなければ例外で中断し、アプリは起動しない。
  kubectl(['delete', 'job', 'zoo2-migrate', '-n', namespace, '--ignore-not-found', '--wait=true']);
  kubectl(['apply', '-f', '-'], render('migrate', config));
  kubectl(['wait', '-n', namespace, '--for=condition=complete', 'job/zoo2-migrate', '--timeout=600s']);

  // 5. 開発用ユーザーを投入する。SQL は infra 側の ConfigMap にある。
  const seed = json(['get', 'configmap', 'preview-seed', '-n', namespace]).data['seed.sql'];
  kubectl(['exec', '-i', '-n', namespace, 'postgres', '--', 'psql', '-v', 'ON_ERROR_STOP=1', '-U', 'zoo2', '-d', 'zoo2'], seed);

  // 6. アプリを起動する。マイグレーション中に PR が更新されていれば、ここで止めて次の実行に任せる。
  if (!currentPR(await getPR(), repository, config.sha)) { core.notice('PR changed during migration; next run will deploy it'); return; }
  kubectl(['apply', '-f', '-'], render('app', config));
  // 中身が同じマニフェストを apply しても、手順4で0にした replica は戻らない。明示的に1へ戻す。
  kubectl(['scale', 'deployment', '--all', '-n', namespace, '--replicas=1']);
  for (const name of ['electric', 'backend', 'frontend']) kubectl(['rollout', 'status', `deployment/${name}`, '-n', namespace, '--timeout=300s']);
  // 起動を待つ間に PR が更新されていれば、古い SHA の URL はコメントしない。
  if (currentPR(await getPR(), repository, config.sha)) await comment(github, context, number, `Preview: https://${config.host}\n\nCommit: \`${config.sha}\`\n\n開発用ユーザー切替が有効です。DB は一時データです。更新から7日後に自動回収します。`);
}

// 最後のデプロイから7日以上経ったか。updated-at がない、または読めない Namespace は期限切れとみなさない。
export function expired(ns, now = Date.now()) {
  const updated = Date.parse(ns.metadata.annotations?.['zoo2.zli.dev/updated-at']);
  return Number.isFinite(updated) && now - updated >= 7 * 86400000;
}

// Preview の Namespace を回収する。
// - PR close（pull_request_target）: close された PR の Namespace だけを対象にする。再 open 済みなら残す
// - 日次・手動実行: close 済み PR の Namespace と、TTL 切れの Namespace を削除する
export async function cleanup({github, context, core}) {
  const owner = `${context.repo.owner}.${context.repo.repo}`;
  const namespaces = json(['get', 'namespaces', '-l', `${managedLabel}=true`]).items;
  // 1つの Namespace が削除できなくても残りの回収は続ける。
  // 途中で止めると後ろの環境が残り続け、同時稼働の上限に達したまま新しい Preview を作れなくなるため。
  const failed = [];
  for (const ns of namespaces.filter(n => isManaged(n, owner))) {
    const number = Number(ns.metadata.name.slice('zoo2-pr-'.length));
    if (context.eventName === 'pull_request_target' && number !== context.payload.pull_request.number) continue;
    try {
      const {data: pr} = await github.rest.pulls.get({...context.repo, pull_number: number});
      // open の PR は、日次・手動実行で TTL を過ぎている場合だけ削除する。
      if (pr.state === 'open' && (context.eventName === 'pull_request_target' || !expired(ns))) continue;
      kubectl(['delete', 'namespace', ns.metadata.name, '--wait=true', '--timeout=180s']);
      await comment(github, context, number, `Preview 環境を削除しました（${pr.state === 'closed' ? 'PR 終了' : '更新から7日経過'}）。再作成するには PR を更新し、Publish images を成功させてください。`);
    } catch (error) {
      core.error(`${ns.metadata.name}: ${error.stderr?.trim() || error.message}`);
      failed.push(ns.metadata.name);
    }
  }
  // 失敗した Namespace があればまとめてエラーにし、ジョブを失敗として残す。
  if (failed.length) throw new Error(`Failed to reclaim: ${failed.join(', ')}`);
}
