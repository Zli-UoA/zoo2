import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { resolve, join } from 'node:path';
import { EventEmitter } from 'node:events';

const source = readFileSync(new URL('./run.mjs', import.meta.url), 'utf8');
const mainSource = source.slice(source.indexOf('export async function main'), source.indexOf('\nif (process.argv[1]')).replace('export async function', 'async function');
function deferred() {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
const flush = async () => { for (let i = 0; i < 12; i++) await Promise.resolve(); };

function harness({ token = Promise.resolve({ stdout: 'テスト用' }), patch = async () => ({ stdout: '{}' }) } = {}) {
  const config = { repositoryPath: '/tmp/ios', websocketURL: 'wss://webhook-forwarder.github.com/test', repositoryID: 12, hookID: 1, threadID: '00000000-0000-4000-8000-000000000001', secretFile: '/tmp/secret', ghPath: 'gh', codexPath: 'codex' };
  const state = { lockClosed: false, sockets: [], signals: {}, timers: [], changes: [], received: 0 };
  const requested = deferred();
  class DB { exec() {} close() { state.lockClosed = true; } }
  class WS extends EventEmitter {
    static OPEN = 1; static CLOSED = 3;
    constructor(url, options) { super(); this.options = options; this.readyState = 0; this.closes = 0; state.sockets.push(this); }
    close() { this.closes++; if (this.readyState === WS.CLOSED) return; this.readyState = WS.CLOSED; this.emit('close'); }
    send() {}
  }
  class J { receive() { assert.equal(state.lockClosed, false); state.received++; return false; } }
  const execute = async (path, args, options) => {
    if (path === 'git') return { stdout: 'https://github.com/Zli-UoA/zoo2-ios.git' };
    if (args[0] === 'auth') { assert.equal(options.timeout, 30000); requested.resolve(); return token; }
    state.changes.push(args); return patch();
  };
  // 実際の起動関数へ外部境界だけを差し替え、競合をネットワークや常駐登録なしで再現します。
  const dependencies = {
    readFileSync: path => path === '/tmp/config' ? JSON.stringify(config) : 'x'.repeat(64),
    mkdirSync() {}, statSync: () => ({ mode: 0o600 }), resolve, join, DatabaseSync: DB, WebSocket: WS,
    Journal: J, selectEvent: () => null, dispatch: async () => false, repository: 'Zli-UoA/zoo2-ios', execute,
    process: { umask() {}, once: (event, handler) => { state.signals[event] = handler; } },
    setInterval: () => 0, clearInterval() {}, setTimeout: handler => { state.timers.push(handler); return handler; }, clearTimeout() {},
    console: { log() {}, error() {} }
  };
  const main = new Function(...Object.keys(dependencies), `${mainSource}\nreturn main;`)(...Object.values(dependencies));
  return { state, main, requested: requested.promise };
}

test('認証待ち中に停止した場合は接続も外部変更も開始しない', async () => {
  const token = deferred(); const { state, main, requested } = harness({ token: token.promise });
  const running = main('/tmp/config'); await requested;
  const stopping = state.signals.SIGTERM();
  assert.equal(state.lockClosed, false);
  token.resolve({ stdout: 'テスト用' });
  await Promise.all([running, stopping]);
  assert.equal(state.lockClosed, true); assert.equal(state.sockets.length, 0); assert.equal(state.changes.length, 0);
});

test('古い接続の有効化失敗が新しい接続を閉じない', async () => {
  const first = deferred(); let calls = 0;
  const { state, main } = harness({ patch: () => ++calls === 1 ? first.promise : Promise.resolve({ stdout: '{}' }) });
  await main('/tmp/config'); const a = state.sockets[0]; a.readyState = 1; a.emit('open');
  assert.equal(a.options.handshakeTimeout, 30000);
  a.close(); state.timers.shift()(); await flush();
  const b = state.sockets[1]; b.readyState = 1; b.emit('open');
  first.reject(new Error('古い接続の失敗')); await flush();
  assert.equal(b.closes, 0); assert.equal(state.lockClosed, false);
  await state.signals.SIGTERM(); assert.equal(state.lockClosed, true);
});

test('停止時は古い有効化要求も待ち、受信記録を更新しない', async () => {
  const first = deferred(); let calls = 0;
  const { state, main } = harness({ patch: () => ++calls === 1 ? first.promise : Promise.resolve({ stdout: '{}' }) });
  await main('/tmp/config'); const a = state.sockets[0]; a.readyState = 1; a.emit('open');
  a.close(); state.timers.shift()(); await flush();
  const b = state.sockets[1]; b.readyState = 1; b.emit('open'); await flush();
  const stopping = state.signals.SIGTERM(); await flush();
  b.emit('message', Buffer.from('{}'));
  assert.equal(state.received, 0); assert.equal(state.lockClosed, false);
  first.resolve({ stdout: '{}' }); await stopping;
  b.emit('message', Buffer.from('{}'));
  assert.equal(state.received, 0); assert.equal(state.lockClosed, true);
});
