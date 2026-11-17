import { readdirSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { pathToFileURL } from 'node:url';

// Keep the partition in one place for both planning and execution.
export const slackTests = [
  'app_event_sources_test', 'activitypub_test', 'slack_api_import_test',
  'slack_link_test', 'slack_sync_test', 'e2ee_channels_test', 'e2ee_keys_test',
  'e2ee_plaintext_features_test', 'e2ee_to_device_test',
  // app_event_sources_test と同じく、全イベントを購読する App を入れるので、チャンネルを消すテストと DB を分ける。
  'message_components_test',
];
export const testLeg = name => slackTests.includes(name) ? 'slack' : 'core';

export function cargoCommands(selection, leg, files) {
  if (!['slack', 'core'].includes(leg)) throw new Error('Invalid E2E leg');
  const targets = selection.full ? files.filter(f => f.endsWith('.rs')).map(f => f.slice(0, -3)) : selection.targets;
  if (!Array.isArray(targets) || targets.some(t => !/^[a-zA-Z0-9_]+$/.test(t))) throw new Error('Invalid test targets');
  const selected = targets.filter(t => testLeg(t) === leg).sort();
  if (!selected.length) {
    if (selection.full) throw new Error('Full E2E partition is empty');
    return [];
  }
  const commands = [['test', ...selected.flatMap(t => ['--test', t])]];
  if (selection.full && leg === 'slack') commands.push(['test', '--lib', 'activitypub::', '--', '--include-ignored', '--test-threads=1']);
  return commands;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const selection = JSON.parse(process.env.TEST_SELECTION);
  const commands = cargoCommands(selection, process.env.LEG, readdirSync('tests'));
  console.log(`Backend Test Selection: ${JSON.stringify(selection)}; leg=${process.env.LEG}`);
  for (const args of commands) {
    const { status } = spawnSync('cargo', args, { stdio: 'inherit' });
    if (status !== 0) { process.exitCode = status ?? 1; break; }
  }
}
