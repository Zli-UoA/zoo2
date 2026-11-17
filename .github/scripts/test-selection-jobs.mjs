import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';
import { importedNames, isScanned, toKebab } from '../../desktop/scripts/collect-lucide-icons.mjs';

const root = fileURLToPath(new URL('../../', import.meta.url));
const sameIcons = (a, b) => {
  const names = source => [...new Set(importedNames(source).map(toKebab))].sort().join('\n');
  return names(a) === names(b);
};

// Package path dependencies and desktop's actual include_str!/icon-scanner
// inputs, rather than running every Rust client for every React edit.
export function selectOtherJobs(eventName, changes, { base, read = (ref, path) => execFileSync('git', ['show', `${ref}:${path}`], { cwd: root, encoding: 'utf8', timeout: 30000 }) } = {}) {
  const full = () => ({ clients: { tui: true, desktop: true }, bots: { bot: true, sdk: true, image: true }, storybookPreview: true });
  if (eventName !== 'pull_request' || !changes?.length || changes.some(c => !['A', 'M'].includes(c.status))) return full();
  const result = { clients: { tui: false, desktop: false }, bots: { bot: false, sdk: false }, storybookPreview: false };
  for (const { path, status } of changes) {
    if (path.startsWith('.github/') || path === 'rust-toolchain.toml' || path === 'flake.nix') return full();
    if (/^(client-core|client-types)\//.test(path)) result.clients = { tui: true, desktop: true };
    else if (path.startsWith('tui/')) result.clients.tui = true;
    else if (path.startsWith('desktop/')) result.clients.desktop = true;
    else if (path.startsWith('crates/zoo-crypto/')) { result.clients.tui = true; result.bots.sdk = true; }
    else if (path.startsWith('crates/zoo-crypto-wasm/')) result.bots.sdk = true;
    else if (path.startsWith('apps/github-bot/')) result.bots.bot = true;
    else if (path.startsWith('examples/e2ee-echo-bot/')) result.bots.sdk = true;
    else if (path.startsWith('sdk/')) {
      if (path === 'sdk/src/index.test.ts') result.bots.bot = true;
      else if (/^sdk\/src\/(e2ee|testing)(\.|\/)/.test(path)) result.bots.sdk = true;
      else result.bots = { bot: true, sdk: true };
    } else if (path.startsWith('frontend/')) {
      if (!/\.(test|spec)\.tsx?$/.test(path) && !path.startsWith('frontend/src/test/') && !path.startsWith('frontend/e2e/')) result.storybookPreview = true;
      if (path === 'frontend/src/styles.css' || path.startsWith('frontend/src/lib/i18n/locales/')) result.clients.desktop = true;
      else if (path.startsWith('frontend/src/') && isScanned(resolve(root, path))) {
        try {
          const before = status === 'A' ? '' : read(base, path);
          if (!sameIcons(before, read('HEAD', path))) result.clients.desktop = true;
        } catch { result.clients.desktop = true; }
      }
    } else if (!/^(backend|infra|docs|crates)\//.test(path) && !/\.(md|txt)$/.test(path)) return full();
  }
  result.bots.image = result.bots.bot && changes.some(c => !c.path.startsWith('apps/github-bot/test/') && !/\.test\.ts$/.test(c.path));
  return result;
}
