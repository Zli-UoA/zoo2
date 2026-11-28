import { readdirSync, readFileSync } from 'node:fs';
import { execFileSync, spawnSync } from 'node:child_process';
import { fileURLToPath, pathToFileURL } from 'node:url';

import { readChanges, testBaseSha, usesSelectiveTests, workflowChanges } from './test-selection-diff.mjs';

const root = fileURLToPath(new URL('../../', import.meta.url));
const analyzer = fileURLToPath(new URL('./rust-test-selection/Cargo.toml', import.meta.url));
const testFilePattern = /^backend\/tests\/([a-zA-Z0-9_]+_test)\.rs$/;

// Keep the partition in one place for both planning and execution.
export const slackTests = [
  'app_event_sources_test', 'activitypub_test', 'slack_api_import_test',
  'slack_link_test', 'slack_sync_test', 'e2ee_channels_test', 'e2ee_keys_test',
  'e2ee_plaintext_features_test', 'e2ee_to_device_test',
  // app_event_sources_test と同じく、全イベントを購読する App を入れるので、チャンネルを消すテストと DB を分ける。
  'message_components_test',
];
export const testLeg = name => slackTests.includes(name) ? 'slack' : 'core';

// These routes have dedicated integration coverage. Shared routes, support code,
// migrations and unclassified paths keep both partitions.
export const routeTestTargets = {
  'backend/src/routes/apns.rs': ['apns_test'],
  'backend/src/routes/bubble_chat.rs': ['bubble_chat_test'],
  'backend/src/routes/calls.rs': ['calls_test'],
  'backend/src/routes/channel_pins.rs': ['channel_pins_test'],
  'backend/src/routes/channel_webhooks.rs': ['channel_webhooks_test', 'e2ee_convert_test', 'apps_migration_test'],
  'backend/src/routes/push.rs': ['apns_test', 'push_test'],
  'backend/src/routes/typing.rs': ['typing_test', 'app_event_sources_test'],
};

export function cargoCommands(selection, leg, files, cases = {}) {
  if (!['slack', 'core'].includes(leg)) throw new Error('Invalid E2E leg');
  const targets = selection.full ? files.filter(f => f.endsWith('.rs')).map(f => f.slice(0, -3)) : selection.targets;
  if (!Array.isArray(targets) || targets.some(t => !/^[a-zA-Z0-9_]+$/.test(t))) throw new Error('Invalid test targets');
  const selected = targets.filter(t => testLeg(t) === leg).sort();
  if (!selected.length) {
    if (selection.full) throw new Error('Full E2E partition is empty');
    return [];
  }
  const wholeTargets = selected.filter(target => !Object.hasOwn(cases, target));
  const commands = wholeTargets.length ? [['test', ...wholeTargets.flatMap(target => ['--test', target])]] : [];
  for (const target of selected.filter(target => Object.hasOwn(cases, target))) {
    commands.push(['test', '--test', target, '--', '--exact', ...cases[target]]);
  }
  if (selection.full && leg === 'slack') commands.push(['test', '--lib', 'activitypub::', '--', '--include-ignored', '--test-threads=1']);
  return commands;
}

export function rustCaseFilters({ selection, leg, base, changes, git = execFileSync, run = spawnSync, log = console.log }) {
  if (selection.full || selection.source || !changes?.length ||
      !changes.every(({ status, path }) => status === 'M' && testFilePattern.test(path))) return {};
  const filters = {};
  const options = { cwd: root, encoding: 'utf8', timeout: 30000 };
  for (const { path } of changes) {
    const [, target] = testFilePattern.exec(path);
    if (!selection.targets.includes(target) || testLeg(target) !== leg) continue;
    try {
      const before = git('git', ['show', `${base}:${path}`], options);
      const after = git('git', ['show', `HEAD:${path}`], options);
      const result = run('cargo', ['run', '--quiet', '--locked', '--manifest-path', analyzer], {
        cwd: root, encoding: 'utf8', input: `${before}\0${after}`, timeout: 120000,
      });
      const names = result.stdout?.trim().split('\n');
      if (result.status === 0 && names?.length && names.every(name => /^[A-Za-z_][A-Za-z0-9_]*$/.test(name))) {
        filters[target] = names;
      } else {
        log(`Rust case analysis unavailable or shared code changed in ${target}; running the whole test target`);
      }
    } catch {
      log(`Could not analyze ${target}; running the whole test target`);
    }
  }
  return filters;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const selection = JSON.parse(process.env.TEST_SELECTION);
  const leg = process.env.LEG;
  let cases = {};
  const eventName = process.env.GITHUB_EVENT_NAME;
  if (usesSelectiveTests(eventName) && !selection.full && !selection.source) {
    try {
      const event = JSON.parse(readFileSync(process.env.GITHUB_EVENT_PATH, 'utf8'));
      const base = testBaseSha(eventName, event);
      const changes = workflowChanges('ci-backend', readChanges(base));
      cases = rustCaseFilters({ selection, leg, base, changes });
    } catch {
      console.log('Rust case analysis could not read the diff; running the selected test targets in full');
    }
  }
  const commands = cargoCommands(selection, leg, readdirSync('tests'), cases);
  console.log(`Rust Test Cases: ${JSON.stringify(cases)}`);
  console.log(`Backend Test Selection: ${JSON.stringify(selection)}; leg=${process.env.LEG}`);
  for (const args of commands) {
    const { status } = spawnSync('cargo', args, { stdio: 'inherit' });
    if (status !== 0) { process.exitCode = status ?? 1; break; }
  }
}
