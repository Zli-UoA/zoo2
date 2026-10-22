// Publish images（cd-images.yaml）の plan ジョブから呼ぶ。ビルドするコンポーネントと、付けるイメージタグを決める。
export const components = ['frontend', 'backend', 'migrate'];

// main の昇格で3つのイメージに共通して付けるリリースタグ。zoo2-infra の Flux Operator が semver で最新を選ぶので、
// ワークフローの実行番号を patch に入れる。実行番号は実行ごとに増え、同じ実行の再実行では変わらない。
export const releaseTag = runNumber => `0.0.${runNumber}`;

// 昇格で付けるタグを、付ける順に返す。各要素は [イメージ, 付けるタグ, 元のタグ]。
// latest・preview を先に付け替え、リリースタグは最後に付ける。zoo2-infra は migrate のリリースタグだけを見るので、
// migrate を最後にして、そのタグが見えた時点でほかの2つにも同じタグが付いているようにする。
// Flux は3つすべてに同じタグを使うため、3つすべてをビルドした場合だけリリースタグを付ける。
export function promotionAliases(selected, sha, release) {
  const image = component => `ghcr.io/zli-uoa/zoo2-${component}`;
  const aliases = selected.flatMap(component => [
    [image(component), 'latest', sha],
    ...(component === 'frontend' ? [[image(component), 'preview', `preview-${sha}`]] : []),
  ]);
  if (release && components.every(component => selected.includes(component))) {
    const ordered = [...selected].sort((a, b) => Number(a === 'migrate') - Number(b === 'migrate'));
    aliases.push(...ordered.map(component => [image(component), release, sha]));
  }
  return aliases;
}

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

// ビルド計画を作り、ジョブの出力に渡す。
// - main への push・手動実行: 全コンポーネントをビルドし、タグは `<SHA>`
// - PR: 変更のあるコンポーネントだけをビルドする。タグは全コンポーネントで `pr-<番号>-<SHA>` にそろえ、
//   変更のないコンポーネントには、昇格済み main コミットの不変タグ（frontend は Preview 用ビルドの `preview-<SHA>`）を元に別名を付ける。
//   site1 の Flux は PR 番号と head SHA だけからタグを組み立てるため、3つのタグがそろっている必要がある。
//   可変タグの latest・preview を元にしないのは、別名の付与と main の昇格が前後しても3つを同じコミットにそろえるため。
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
  const tags = Object.fromEntries(components.map(c => [c, pr ? `pr-${pr.number}-${sha}` : sha]));
  // 変更のないコンポーネントと、別名の元にするタグ。main では全コンポーネントをビルドするので空になる。
  const unchanged = components.filter(c => !selected.includes(c));
  const mainSha = unchanged.length ? await promotedMainSha(github, context.repo.owner) : null;
  const aliases = unchanged.map(component => ({component, source: component === 'frontend' ? `preview-${mainSha}` : mainSha}));
  core.setOutput('enabled', 'true');
  core.setOutput('matrix', JSON.stringify({include: selected.map(component => ({component}))}));
  core.setOutput('has_images', String(selected.length > 0));
  core.setOutput('sha', sha);
  core.setOutput('tags', JSON.stringify(tags));
  core.setOutput('aliases', JSON.stringify(aliases));
  core.setOutput('pr', pr ? String(pr.number) : '');
  // PR は昇格しないので、リリースタグを付けない。
  core.setOutput('release', pr ? '' : releaseTag(context.runNumber));
  await core.summary.addHeading('Image references').addCodeBlock(JSON.stringify({tags, aliases}, null, 2), 'json').write();
}
