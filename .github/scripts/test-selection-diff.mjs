import { readdirSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { execFileSync } from 'node:child_process';
import { isAndroidFrontendOnly } from './frontend-test-selection-paths.mjs';
import { pathMatcher, testWorkflowPaths } from './merge-queue-changes.mjs';
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
  const matches = pathMatcher(testWorkflowPaths(workflow, source));
  return changes.filter(change => matches(change.path));
}

export const isFrontendLocale = path => /^frontend\/src\/lib\/i18n\/locales\/[^/]+\.json$/.test(path);

export function isLightweightChange({ path, status }) {
  return path === 'frontend/README.md' || (status === 'M' && isFrontendLocale(path));
}

export function isFrontendTestInput(path) {
  return !isAndroidFrontendOnly(path) &&
    !/^(frontend-compose\/|docs\/images\/|docs\/.+\.md$|\.agents\/|\.github\/workflows\/ci-android\.yaml$)/.test(path);
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

function translationEntries(value, file, key = '', entries = new Map()) {
  if (typeof value === 'string' && key) {
    if (!value.trim()) throw new Error(`${file}: empty translation for ${key}`);
    const variables = [...value.matchAll(/{{\s*-?\s*([\w.]+)(?:\s*,[^{}]+)?\s*}}/g)];
    if (value.replace(/{{\s*-?\s*[\w.]+(?:\s*,[^{}]+)?\s*}}/g, '').match(/{{|}}/)) {
      throw new Error(`${file}: invalid interpolation in ${key}`);
    }
    entries.set(key, [...new Set(variables.map(([, name]) => name))].sort().join(','));
    return entries;
  }
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    throw new Error(`${file}: expected translation strings or nested objects at ${key || 'root'}`);
  }
  for (const [name, child] of Object.entries(value)) {
    translationEntries(child, file, key ? `${key}.${name}` : name, entries);
  }
  return entries;
}

// Removing a key or changing its interpolation contract can affect callers.
export function translationOnlyChange(before, after, file) {
  const oldEntries = translationEntries(JSON.parse(before), file);
  const newEntries = translationEntries(JSON.parse(after), file);
  return [...oldEntries].every(([key, variables]) => newEntries.get(key) === variables);
}

export function validateTranslationDictionaries(dictionaries) {
  const reference = translationEntries(JSON.parse(dictionaries['ja.json']), 'ja.json');
  for (const [file, source] of Object.entries(dictionaries)) {
    const entries = translationEntries(JSON.parse(source), file);
    for (const key of entries.keys()) {
      if (!reference.has(key)) throw new Error(`${file}: unexpected translation key ${key}`);
    }
    for (const [key, variables] of reference) {
      if (entries.get(key) !== variables) throw new Error(`${file}: missing key or mismatched interpolation for ${key}`);
    }
  }
}

export function validateChangedTranslations(changes) {
  if (!changes.some(({ path }) => isFrontendLocale(path))) return;
  const directory = resolve(root, 'frontend/src/lib/i18n/locales');
  const files = readdirSync(directory).filter(name => name.endsWith('.json'));
  validateTranslationDictionaries(Object.fromEntries(files.map(file => [file, readFileSync(resolve(directory, file), 'utf8')])));
}

export function classifyTranslationChanges(base, changes, { git = execFileSync, read = readFileSync } = {}) {
  return changes.map(change => {
    if (!isFrontendLocale(change.path) || !['A', 'M'].includes(change.status)) return change;
    const before = change.status === 'A' ? '{}' : git('git', ['show', `${base}:${change.path}`], { cwd: root, encoding: 'utf8' });
    const after = read(resolve(root, change.path), 'utf8');
    return translationOnlyChange(before, after, change.path) ? change : { ...change, status: 'T' };
  });
}
