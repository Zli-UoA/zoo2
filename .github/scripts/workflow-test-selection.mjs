import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { parseHunks } from './browser-test-selection.mjs';

const root = fileURLToPath(new URL('../../', import.meta.url));
function sections(source, rootKey) {
  const lines = source.split('\n');
  const rootIndex = lines.findIndex(line => line === `${rootKey}:`);
  if (rootIndex < 0) return null;
  const result = [];
  for (let i = rootIndex + 1; i < lines.length; i++) {
    if (/^\S/.test(lines[i]) && !lines[i].startsWith('#')) { if (result.length) result.at(-1).end = i; break; }
    const id = lines[i].match(/^  ([\w-]+):\s*(?:#.*)?$/)?.[1];
    if (id) { if (result.length) result.at(-1).end = i; result.push({ id, start: i + 1, end: lines.length, text: '' }); }
  }
  for (const section of result) section.text = lines.slice(section.start - 1, section.end).join('\n');
  return result;
}

export function changedSections(before, after, diff, rootKey = 'jobs') {
  const oldSections = sections(before, rootKey), newSections = sections(after, rootKey);
  if (!oldSections || !newSections) return null;
  if (rootKey === 'jobs' && oldSections.map(s => s.id).sort().join() !== newSections.map(s => s.id).sort().join()) return null;
  const selected = new Set();
  const hunks = parseHunks(diff);
  if (before !== after && !hunks.length) return null;
  for (const hunk of hunks) for (const [key, source, ranges] of [['before', before, oldSections], ['after', after, newSections]]) {
    const lines = source.split('\n'), span = hunk[key];
    for (let line = span.start; line < span.start + span.count; line++) {
      const raw = lines[line - 1] ?? '', text = raw.trim();
      // Indented '#' lines may be data inside a shell here-document, not YAML
      // comments. Only skip unambiguous comments outside scalar job content.
      if (!text || (text.startsWith('#') && raw.length - raw.trimStart().length <= 2)) continue;
      const range = ranges.find(r => line >= r.start && line <= r.end);
      if (!range) return null; // Global env, triggers, defaults or unknown structure.
      selected.add(range.id);
    }
  }
  return selected;
}

export function jobClosure(source, changed) {
  const jobs = sections(source, 'jobs');
  if (!jobs || changed === null) return null;
  const dependencies = new Map();
  for (const job of jobs) {
    const lines = job.text.split('\n');
    const indexes = lines.flatMap((line, i) => /^    needs:/.test(line) ? [i] : []);
    if (indexes.length > 1) return null;
    let needs = [];
    if (indexes.length) {
      const i = indexes[0];
      const value = lines[i].replace(/^    needs:\s*/, '').replace(/\s+#.*$/, '').trim();
      if (value) {
        const match = value.match(/^(?:\[([^\]]*)\]|([\w-]+|"[\w-]+"|'[\w-]+'))$/);
        if (!match) return null;
        const values = match[1] === undefined ? [match[2]] : match[1].split(',');
        if (values.length === 1 && !values[0].trim()) needs = [];
        else {
          if (values.some(v => !/^["']?[\w-]+["']?$/.test(v.trim()))) return null;
          needs = values.map(v => v.trim().replace(/^["']|["']$/g, ''));
        }
      } else {
        for (let j = i + 1; j < lines.length; j++) {
          const line = lines[j];
          if (!line.trim() || line.trimStart().startsWith('#')) continue;
          if (/^ {0,4}\S/.test(line)) break;
          const match = line.match(/^      - ([\w-]+|"[\w-]+"|'[\w-]+')\s*(?:#.*)?$/);
          if (!match) return null;
          needs.push(match[1].replace(/^["']|["']$/g, ''));
        }
        if (!needs.length) return null;
      }
    }
    if (needs.some(id => !jobs.some(j => j.id === id))) return null;
    dependencies.set(job.id, needs);
  }
  const selected = new Set(changed);
  if ([...selected].some(id => !dependencies.has(id))) return null;
  // First select consumers of modified producers; then their prerequisites.
  // Do not select every consumer of a prerequisite that itself did not change.
  let size;
  do { size = selected.size; for (const [id, needs] of dependencies) if (needs.some(n => selected.has(n))) selected.add(id); } while (size !== selected.size);
  do { size = selected.size; for (const id of selected) for (const need of dependencies.get(id) ?? []) selected.add(need); } while (size !== selected.size);
  return selected;
}

export function workflowJobs(workflow, changes, { base, head = 'HEAD', git = execFileSync } = {}) {
  if (!workflow || !/^[\w-]+$/.test(workflow) || !changes?.length || changes.some(c => !['A', 'M'].includes(c.status))) return null;
  if (!/^[a-f0-9]{40}$/.test(base ?? '') || (head !== 'HEAD' && !/^[a-f0-9]{40}$/.test(head))) return null;
  if (!changes.every(c => /^\.github\/workflows\/ci-[\w-]+\.yaml$/.test(c.path) || /^infra\/compose\.ci-cache(?:-write)?\.yaml$/.test(c.path))) return null;
  const options = { cwd: root, encoding: 'utf8', timeout: 30000 };
  const show = (ref, path) => git('git', ['show', `${ref}:${path}`], options);
  const diff = path => git('git', ['diff', '--no-ext-diff', '--no-renames', '--unified=0', base, head, '--', path], options);
  const path = `.github/workflows/${workflow}.yaml`;
  try {
    const source = show(head, path);
    const selected = changes.some(c => c.path === path) ? changedSections(show(base, path), source, diff(path)) : new Set();
    if (selected === null) return null;
    for (const c of changes.filter(c => c.path.startsWith('infra/'))) {
      const services = changedSections(show(base, c.path), show(head, c.path), diff(c.path), 'services');
      const consumers = { 'ci-backend': { backend: 'deploy-image' }, 'ci-frontend': { frontend: 'container', proxy: 'proxy' } }[workflow] ?? {};
      const unknownService = services && [...services].some(service => !['frontend', 'backend', 'proxy'].includes(service));
      for (const [service, job] of Object.entries(consumers)) if (services === null || unknownService || services.has(service)) selected.add(job);
    }
    return jobClosure(source, selected);
  } catch { return null; }
}

export function restrictWorkflow(selection, workflow, jobs) {
  if (jobs === null) return selection;
  if (workflow === 'ci-frontend') {
    for (const key of ['container', 'proxy']) selection.frontend[key] = jobs.has(key);
    selection.frontend.unit = selection.frontend.pwa = jobs.has('unit-test');
    for (const key of ['browser', 'vrt', 'storybook', 'avatar', 'offline', 'i18n', 'layout', 'frontend-selection', 'catalog']) selection.frontend[key] = jobs.has('browser-test');
    selection.frontendBuild = { check: jobs.has('check'), build: jobs.has('build'), wasm: jobs.has('crypto-wasm') };
  } else if (workflow === 'ci-backend') {
    Object.assign(selection.backend, { check: jobs.has('check'), deploy: jobs.has('deploy-image'), run: jobs.has('e2e'), emoji: jobs.has('shared-emoji-projection') });
  } else if (workflow === 'ci-desktop-tauri') selection.desktopTauri = { check: jobs.has('check'), macos: jobs.has('macos') };
  else if (workflow === 'ci-tui') selection.clients = { tui: jobs.has('check'), desktop: jobs.has('desktop') };
  else if (workflow === 'ci-github-bot') selection.bots = { bot: jobs.has('test'), sdk: jobs.has('sdk-e2ee'), image: jobs.has('test') };
  else if (workflow === 'ci-frontend-auth-e2e') selection.frontend.auth = jobs.has('oidc-e2e');
  else if (workflow === 'ci-frontend-e2ee-e2e') selection.frontend.e2ee = jobs.has('e2ee-e2e');
  return selection;
}
