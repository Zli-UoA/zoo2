export const components = ['frontend', 'backend', 'migrate'];
export function changedComponents(files) {
  const shared = files.some(f => f.startsWith('.github/scripts/') || f.startsWith('.github/actions/') || f === '.github/workflows/cd-images.yaml' || f === '.dockerignore');
  return components.filter(c => shared || files.some(f =>
    f.startsWith(`infra/Dockerfile-${c}-deploy`) ||
    (c === 'migrate' ? f.startsWith('infra/db/') : f.startsWith(`${c}/`)) ||
    (c === 'frontend' && f === 'infra/frontend-nginx.conf')));
}
export default async function plan({github, context, core}) {
  const pr = context.payload.pull_request;
  if (pr && pr.head.repo.full_name !== `${context.repo.owner}/${context.repo.repo}`) {
    core.setOutput('enabled', 'false');
    return;
  }
  let selected = components;
  if (pr) {
    const {data: current} = await github.rest.pulls.get({...context.repo, pull_number: pr.number});
    if (current.state !== 'open' || current.head.sha !== pr.head.sha) {
      core.setOutput('enabled', 'false');
      return;
    }
    const files = await github.paginate(github.rest.pulls.listFiles, {...context.repo, pull_number: pr.number, per_page: 100});
    // GitHub truncates this endpoint at 3000 files. Build everything rather than miss changes.
    selected = files.length >= 3000 ? components : changedComponents(files.flatMap(f => [f.filename, f.previous_filename].filter(Boolean)));
  }
  const sha = pr?.head.sha ?? context.sha;
  const tags = Object.fromEntries(components.map(c => [c, selected.includes(c) ? (pr ? `pr-${pr.number}-${sha}` : sha) : c === 'frontend' ? 'preview' : 'latest']));
  core.setOutput('enabled', 'true');
  core.setOutput('matrix', JSON.stringify({include: selected.map(component => ({component}))}));
  core.setOutput('has_images', String(selected.length > 0));
  core.setOutput('sha', sha);
  core.setOutput('tags', JSON.stringify(tags));
  await core.summary.addHeading('Image references').addCodeBlock(JSON.stringify(tags, null, 2), 'json').write();
}
