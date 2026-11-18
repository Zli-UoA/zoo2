import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {mkdtempSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import test from 'node:test';
import {fileURLToPath} from 'node:url';

const script = fileURLToPath(new URL('./build-macos.sh', import.meta.url));
const config = '{"bundle":{"createUpdaterArtifacts":true},"plugins":{"updater":{"pubkey":"テスト用 公開鍵"}}}';
const target = '/Users/runner/work/zoo2/zoo2/desktop-tauri/src-tauri/target/universal-apple-darwin/release/bundle/macos/zoo2.app/Contents/MacOS/zoo2-desktop-tauri';
const signing = `Signing with identity "Developer ID Application: テスト"\nSigning ${target}\n`;
const timestamp = `${target}: A timestamp was expected but was not found.\n`;
const codesign = 'Error failed to bundle project: failed codesign application: failed to run command codesign: failed to sign app\n';
const temporaryFailure = {status: 1, stderr: signing + timestamp + codesign};

function run(t, results, {teeFailure = false, signal} = {}) {
  const dir = mkdtempSync(join(tmpdir(), 'zoo2-macos-build-test-'));
  t.after(() => rmSync(dir, {recursive: true, force: true}));
  const bin = join(dir, 'bin');
  const logs = join(dir, 'logs');
  mkdirSync(bin);
  mkdirSync(logs);
  writeFileSync(join(dir, 'calls.json'), '[]');
  writeFileSync(join(dir, 'sleeps'), '');
  writeFileSync(join(bin, 'cargo'), `#!/usr/bin/env node
const fs = require('node:fs');
const calls = JSON.parse(fs.readFileSync('calls.json', 'utf8'));
const result = JSON.parse(process.env.TEST_RESULTS)[calls.length] ?? {status: 99};
calls.push(process.argv.slice(2));
fs.writeFileSync('calls.json', JSON.stringify(calls));
fs.writeFileSync('artifact', '配布物');
fs.writeFileSync('log-modes.json', JSON.stringify(fs.readdirSync(process.env.TMPDIR).map(name => fs.statSync(process.env.TMPDIR + '/' + name).mode & 0o777)));
fs.writeSync(1, result.stdout ?? 'ビルド実行\\n');
fs.writeSync(2, result.stderr ?? '');
process.exitCode = result.status;
`, {mode: 0o755});
  // シグナルの送信先は、このテストが起動した再試行スクリプトだけに限定する。
  writeFileSync(join(bin, 'sleep'), '#!/usr/bin/env bash\nprintf "%s\\n" "$1" >> sleeps\nif [ -n "$TEST_SIGNAL" ]; then kill -s "$TEST_SIGNAL" "$PPID"; fi\n', {mode: 0o755});
  if (teeFailure) {
    writeFileSync(join(bin, 'tee'), '#!/usr/bin/env bash\ncat > /dev/null\nexit 74\n', {mode: 0o755});
  }
  const result = spawnSync('bash', [script, config], {
    cwd: dir,
    env: {...process.env, PATH: `${bin}:${process.env.PATH}`, TMPDIR: logs, TEST_RESULTS: JSON.stringify(results), TEST_SIGNAL: signal ?? ''},
    encoding: 'utf8',
    timeout: 10000,
  });
  assert.ifError(result.error);
  assert.equal(result.signal, null);
  const calls = JSON.parse(readFileSync(join(dir, 'calls.json'), 'utf8'));
  for (const args of calls) {
    assert.deepEqual(args, ['tauri', 'build', '--target', 'universal-apple-darwin', '--bundles', 'app,dmg', '--config', config, '--', '--locked']);
  }
  assert.deepEqual(readdirSync(logs), [], '成功・失敗のどちらでも一時ログを消す');
  assert.deepEqual(JSON.parse(readFileSync(join(dir, 'log-modes.json'), 'utf8')), [0o600], '一時ログは所有者だけが読み書きできる');
  assert.equal(statSync(join(dir, 'artifact')).mode & 0o777, 0o666 & ~process.umask(), '配布物の権限は呼び出し元から変更しない');
  return {...result, attempts: calls.length, sleeps: readFileSync(join(dir, 'sleeps'), 'utf8')};
}

test('初回成功なら待機も再試行もしない', t => {
  const result = run(t, [{status: 0, stdout: '配布物を作成しました\n'}]);
  assert.equal(result.status, 0);
  assert.equal(result.attempts, 1);
  assert.equal(result.sleeps, '');
  assert.match(result.stdout, /配布物を作成しました/);
});

for (const failures of [1, 2]) {
  test(`タイムスタンプ取得が ${failures} 回失敗しても次の成功を返す`, t => {
    const result = run(t, [...Array(failures).fill(temporaryFailure), {status: 0}]);
    assert.equal(result.status, 0);
    assert.equal(result.attempts, failures + 1);
    assert.equal(result.sleeps, failures === 1 ? '30\n' : '30\n60\n');
    assert.equal(result.stdout.split(timestamp).length - 1, failures, '失敗した試行のログも表示する');
  });
}

test('3 回とも失敗したら追加の待機をせず最後の終了コードを返す', t => {
  const result = run(t, [temporaryFailure, temporaryFailure, {...temporaryFailure, status: 42}]);
  assert.equal(result.status, 42);
  assert.equal(result.attempts, 3);
  assert.equal(result.sleeps, '30\n60\n');
});

for (const [name, stderr] of [
  ['証明書', 'error: The specified item could not be found in the keychain.\n' + codesign],
  ['公証', 'Error failed to notarize app: Invalid\n'],
  ['ビルド', 'error: could not compile zoo2-desktop-tauri\n'],
  ['タイムスタンプに言及した別処理', timestamp + 'error: 別処理が失敗しました\n'],
  ['別のタイムスタンプ処理', 'zoo2.app: timestamp service is not available\n' + codesign],
  ['先行するタイムスタンプ警告と現在の署名', 'warning: ' + timestamp + signing + codesign],
  ['別ファイルのタイムスタンプと現在の署名', signing + timestamp + 'Signing /another.app\n' + codesign],
]) {
  test(`${name}のエラーは直ちに失敗させる`, t => {
    const result = run(t, [{status: 17, stderr}]);
    assert.equal(result.status, 17);
    assert.equal(result.attempts, 1);
    assert.equal(result.sleeps, '');
    assert.ok(result.stdout.includes(stderr));
  });
}

test('再試行で別のエラーに変わったら以前のログに影響されず止まる', t => {
  const result = run(t, [temporaryFailure, {status: 23, stderr: codesign}]);
  assert.equal(result.status, 23);
  assert.equal(result.attempts, 2);
  assert.equal(result.sleeps, '30\n');
});

test('中断の終了コードはタイムスタンプエラーがあっても再試行しない', t => {
  const result = run(t, [{...temporaryFailure, status: 130}]);
  assert.equal(result.status, 130);
  assert.equal(result.attempts, 1);
  assert.equal(result.sleeps, '');
});

for (const [signal, status] of [['INT', 130], ['TERM', 143]]) {
  test(`待機中に SIG${signal} を受けたら次のビルドを開始しない`, t => {
    const result = run(t, [temporaryFailure, {status: 0}], {signal});
    assert.equal(result.status, status);
    assert.equal(result.attempts, 1);
    assert.equal(result.sleeps, '30\n');
  });
}

for (const build of [{status: 0}, temporaryFailure]) {
  test(`ログの保存が失敗したらビルドの終了コード ${build.status} にかかわらず失敗させる`, t => {
    const result = run(t, [build], {teeFailure: true});
    assert.equal(result.status, 74);
    assert.equal(result.attempts, 1);
    assert.equal(result.sleeps, '');
  });
}
