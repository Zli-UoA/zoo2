// マージキューで実行する CI を選ぶ。各 workflow の対象パスを読み、
// キューの base からの差分に当てはまるものを true として GITHUB_OUTPUT に書く。
// パスの一覧は各 workflow にだけ書き、merge-queue.yaml には重複させない。
import { execFileSync } from 'node:child_process';
import { appendFileSync, readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';

export const WORKFLOWS = ['ci-backend', 'ci-home-query-budget', 'ci-read-state-worker-budget', 'ci-phone-query-budget', 'ci-search-query-budget', 'ci-components-query-budget', 'ci-frontend', 'ci-frontend-auth-e2e', 'ci-frontend-e2ee-e2e', 'ci-tui', 'ci-desktop-tauri', 'ci-crypto', 'ci-ios', 'ci-github-bot', 'ci-flux-status', 'ci-db-erd', 'terraform-r2', 'ci-android'];

// These workflows are called by PR CI and use the same paths as main.
export const PR_WORKFLOWS = ['ci-backend', 'ci-frontend', 'ci-tui', 'ci-github-bot'];

// 指定した on イベントの paths を読む。既定は pull_request。
export function pullRequestPaths(yaml, event = 'pull_request') {
  const on = yaml.split(/^on:\s*$/m)[1]?.split(/^\S/m)[0] ?? '';
  const eventBlock = on.split(new RegExp(`^  ${event}:\\s*$`, 'm'))[1]?.split(/^  \S/m)[0] ?? '';
  const paths = eventBlock.split(/^    paths:\s*$/m)[1]?.split(/^    \S/m)[0] ?? '';
  return [...paths.matchAll(/^      - ["']?([^"'\n]+?)["']?\s*$/gm)].map(m => m[1]);
}

// GitHub evaluates matching patterns in order, with ! excluding a path.
export function pathMatcher(patterns) {
  const rules = patterns.map(pattern => {
    const exclude = pattern.startsWith('!');
    const glob = exclude ? pattern.slice(1) : pattern;
    const source = glob.split(/(\*\*\/|\*\*|\*)/).map(part => {
      if (part === '**/') return '(?:.*/)?';
      if (part === '**') return '.*';
      if (part === '*') return '[^/]*';
      return part.replace(/[.+?^${}()|[\]\\]/g, '\\$&');
    }).join('');
    return { exclude, regex: new RegExp(`^${source}$`) };
  });
  return path => rules.reduce((included, { exclude, regex }) =>
    regex.test(path) ? !exclude : included, rules.length === 0);
}

export function testWorkflowPaths(workflow, source, eventName) {
  const usePush = PR_WORKFLOWS.includes(workflow) ||
    (workflow === 'ci-frontend-e2ee-e2e' && eventName !== 'pull_request');
  return pullRequestPaths(source, usePush ? 'push' : 'pull_request');
}

export function selectWorkflows(changed, readWorkflow) {
  // merge-queue 自体の変更では呼び出し方が変わるので、すべて実行する。
  const self = ['.github/workflows/merge-queue.yaml', '.github/scripts/merge-queue-changes.mjs'];
  return Object.fromEntries(WORKFLOWS.map(workflow => {
    const matches = pathMatcher(testWorkflowPaths(workflow, readWorkflow(workflow)));
    return [workflow, changed.some(file => self.includes(file) || matches(file))];
  }));
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const base = process.env.BASE_SHA;
  execFileSync('git', ['fetch', '--no-tags', '--depth=1', 'origin', base], { stdio: 'inherit' });
  const changed = execFileSync('git', ['diff', '--no-renames', '--name-only', '-z', base, 'HEAD'], { encoding: 'utf8' }).split('\0').filter(Boolean);
  console.log(`changed:\n${changed.join('\n')}`);
  const selected = selectWorkflows(changed, workflow => readFileSync(`.github/workflows/${workflow}.yaml`, 'utf8'));
  for (const [workflow, run] of Object.entries(selected)) {
    console.log(`${workflow}=${run}`);
    if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, `${workflow}=${run}\n`);
  }
}
