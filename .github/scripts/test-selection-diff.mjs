import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { pathMatcher, pullRequestPaths } from './merge-queue-changes.mjs';
import { fileURLToPath } from 'node:url';
const root = fileURLToPath(new URL('../../', import.meta.url));
const shaPattern = /^[a-f0-9]{40}$/;

export function usesSelectiveTests(eventName) {
  return eventName === 'pull_request' || eventName === 'merge_group';
}

export function testBaseSha(eventName, event) {
  const sha = {
    pull_request: event?.pull_request?.base?.sha,
    merge_group: event?.merge_group?.base_sha,
    push: event?.before,
  }[eventName];
  return typeof sha === 'string' && shaPattern.test(sha) ? sha : undefined;
}

export function workflowChanges(workflow, changes) {
  if (!changes || !/^[\w-]+$/.test(workflow ?? '')) return changes;
  const source = readFileSync(new URL(`../workflows/${workflow}.yaml`, import.meta.url), 'utf8');
  const matches = pathMatcher(pullRequestPaths(source));
  return changes.filter(change => matches(change.path));
}

export function readChanges(base, git = execFileSync) {
  if (!shaPattern.test(base ?? '')) throw new Error('Invalid base SHA');
  const options = { cwd: root, encoding: 'utf8', timeout: 30000 };
  try {
    git('git', ['cat-file', '-e', `${base}^{commit}`], { ...options, stdio: 'pipe' });
  } catch {
    git('git', ['fetch', '--no-tags', '--depth=1', 'origin', base], { ...options, stdio: 'pipe' });
  }
  // Disable rename detection so a rename includes a deletion and triggers a full run.
  const fields = git('git', ['diff', '--no-renames', '--name-status', '-z', base, 'HEAD'], options).split('\0');
  if (fields.pop() !== '' || fields.length % 2) throw new Error('Invalid git diff');
  const changes = [];
  for (let i = 0; i < fields.length; i += 2) changes.push({ status: fields[i], path: fields[i + 1] });
  return changes;
}
