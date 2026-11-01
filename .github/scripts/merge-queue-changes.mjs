// マージキューで実行する CI を選ぶ。各 workflow の on.pull_request.paths を読み、
// キューの base からの差分に当てはまるものを true として GITHUB_OUTPUT に書く。
// パスの一覧は各 workflow にだけ書き、merge-queue.yaml には重複させない。
import { execFileSync } from 'node:child_process';
import { appendFileSync, readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';

export const WORKFLOWS = ['ci-backend', 'ci-frontend', 'ci-frontend-auth-e2e', 'ci-compose', 'ci-tui', 'ci-github-bot', 'ci-flux-status', 'terraform-r2'];

// on: の pull_request: にある paths: の一覧だけを読む。各 workflow はこの形で書かれている。
export function pullRequestPaths(yaml) {
  const on = yaml.split(/^on:\s*$/m)[1]?.split(/^\S/m)[0] ?? '';
  const pullRequest = on.split(/^  pull_request:\s*$/m)[1]?.split(/^  \S/m)[0] ?? '';
  const paths = pullRequest.split(/^    paths:\s*$/m)[1]?.split(/^    \S/m)[0] ?? '';
  return [...paths.matchAll(/^      - ["']?([^"'\n]+?)["']?\s*$/gm)].map(m => m[1]);
}

// GitHub の paths と同じく、** は / を含めて、* は / を除いて一致させる。
export function globToRegExp(glob) {
  const source = glob.split('**').map(part => part.split('*').map(s => s.replace(/[.+?^${}()|[\]\\]/g, '\\$&')).join('[^/]*')).join('.*');
  return new RegExp(`^${source}$`);
}

export function selectWorkflows(changed, readWorkflow) {
  // merge-queue 自体の変更では呼び出し方が変わるので、すべて実行する。
  const self = ['.github/workflows/merge-queue.yaml', '.github/scripts/merge-queue-changes.mjs'];
  return Object.fromEntries(WORKFLOWS.map(workflow => {
    const patterns = [...pullRequestPaths(readWorkflow(workflow)), ...self].map(globToRegExp);
    return [workflow, changed.some(file => patterns.some(p => p.test(file)))];
  }));
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  const base = process.env.BASE_SHA;
  execFileSync('git', ['fetch', '--no-tags', '--depth=1', 'origin', base], { stdio: 'inherit' });
  const changed = execFileSync('git', ['diff', '--name-only', '-z', base, 'HEAD'], { encoding: 'utf8' }).split('\0').filter(Boolean);
  console.log(`changed:\n${changed.join('\n')}`);
  const selected = selectWorkflows(changed, workflow => readFileSync(`.github/workflows/${workflow}.yaml`, 'utf8'));
  for (const [workflow, run] of Object.entries(selected)) {
    console.log(`${workflow}=${run}`);
    if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, `${workflow}=${run}\n`);
  }
}
