// Package path dependencies, rather than running every Rust client for every React edit.
export function selectOtherJobs(eventName, changes) {
  const full = () => ({ clients: { tui: true }, bots: { bot: true, sdk: true, image: true }, storybookPreview: true });
  if (eventName !== 'pull_request' || !changes?.length || changes.some(c => !['A', 'M'].includes(c.status))) return full();
  const result = { clients: { tui: false }, bots: { bot: false, sdk: false }, storybookPreview: false };
  for (const { path } of changes) {
    if (path.startsWith('.github/') || path === 'rust-toolchain.toml' || path === 'flake.nix') return full();
    if (/^(client-core|client-types)\//.test(path) || path.startsWith('tui/')) result.clients.tui = true;
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
    } else if (!/^(backend|infra|docs|crates)\//.test(path) && !/\.(md|txt)$/.test(path)) return full();
  }
  result.bots.image = result.bots.bot && changes.some(c => !c.path.startsWith('apps/github-bot/test/') && !/\.test\.ts$/.test(c.path));
  return result;
}
