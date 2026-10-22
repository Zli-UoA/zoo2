// Publish images（cd-images.yaml）の plan ジョブから呼ぶ。ビルドするコンポーネントと、付けるイメージタグを決める。
export const components = ['frontend', 'backend', 'migrate'];

// 変更されたファイルの一覧から、ビルドが必要なコンポーネントを返す。
// ビルド用のスクリプト・アクション・ワークフロー、.dockerignore が変わった場合は、すべてのコンポーネントをビルドし直す。
// それ以外は、各コンポーネントのソースと Dockerfile の変更を見る（migrate のソースは infra/db/、frontend は nginx 設定も含む）。
export function changedComponents(files) {
  const shared = files.some(f => f.startsWith('.github/scripts/') || f.startsWith('.github/actions/') || f === '.github/workflows/cd-images.yaml' || f === '.dockerignore');
  return components.filter(c => shared || files.some(f =>
    f.startsWith(`infra/Dockerfile-${c}-deploy`) ||
    (c === 'migrate' ? f.startsWith('infra/db/') : f.startsWith(`${c}/`)) ||
    (c === 'frontend' && f === 'infra/frontend-nginx.conf')));
}

// ビルド計画を作り、ジョブの出力に渡す。
// - main への push・手動実行: 全コンポーネントをビルドし、タグは `<SHA>`
// - PR: 変更のあるコンポーネントだけをビルドし、タグは `pr-<番号>-<SHA>`
// fork の PR、close 済みの PR、すでに新しいコミットが push された PR は `enabled=false` にしてビルドしない。
export default async function plan({github, context, core}) {
  const pr = context.payload.pull_request;
  if (pr && pr.head.repo.full_name !== `${context.repo.owner}/${context.repo.repo}`) {
    core.setOutput('enabled', 'false');
    return;
  }
  let selected = components;
  if (pr) {
    // イベントの発生後に PR が close・更新されていれば、古いコミットはビルドしない。
    const {data: current} = await github.rest.pulls.get({...context.repo, pull_number: pr.number});
    if (current.state !== 'open' || current.head.sha !== pr.head.sha) {
      core.setOutput('enabled', 'false');
      return;
    }
    const files = await github.paginate(github.rest.pulls.listFiles, {...context.repo, pull_number: pr.number, per_page: 100});
    // GitHub の変更ファイル一覧は3000件で打ち切られる。変更を見落とさないよう、3000件以上ならすべてビルドする。
    selected = files.length >= 3000 ? components : changedComponents(files.flatMap(f => [f.filename, f.previous_filename].filter(Boolean)));
  }
  const sha = pr?.head.sha ?? context.sha;
  // ビルドしないコンポーネントには参照先として可変タグを書いておく（frontend は Preview 用ビルドの preview、他は latest）。
  const tags = Object.fromEntries(components.map(c => [c, selected.includes(c) ? (pr ? `pr-${pr.number}-${sha}` : sha) : c === 'frontend' ? 'preview' : 'latest']));
  core.setOutput('enabled', 'true');
  core.setOutput('matrix', JSON.stringify({include: selected.map(component => ({component}))}));
  core.setOutput('has_images', String(selected.length > 0));
  core.setOutput('sha', sha);
  core.setOutput('tags', JSON.stringify(tags));
  await core.summary.addHeading('Image references').addCodeBlock(JSON.stringify(tags, null, 2), 'json').write();
}
