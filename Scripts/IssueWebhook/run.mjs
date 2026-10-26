import { readFileSync, mkdirSync, statSync } from 'node:fs';
import { DatabaseSync } from 'node:sqlite';
import { resolve, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import WebSocket from 'ws';
import { Journal, selectEvent, dispatch, repository } from './bridge.mjs';

const execute = promisify(execFile);
export async function main(configPath) {
  process.umask(0o077);
  const config = JSON.parse(readFileSync(configPath, 'utf8'));
  const root = resolve(config.repositoryPath);
  const directory = join(root, '.autodev/webhook');
  mkdirSync(directory, { recursive: true, mode: 0o700 });
  const remote = (await execute('git', ['-C', root, 'remote', 'get-url', 'origin'])).stdout.trim();
  if (![`https://github.com/${repository}.git`, `git@github.com:${repository}.git`].includes(remote)) throw new Error('実行先のリポジトリが一致しません');
  const destination = new URL(config.websocketURL);
  if (destination.protocol !== 'wss:' || !(destination.hostname === 'github.com' || destination.hostname.endsWith('.github.com')) || destination.username || destination.password || destination.port) throw new Error('GitHub以外の接続先は使えません');
  if (!Number.isSafeInteger(config.repositoryID) || !Number.isSafeInteger(config.hookID) || !/^[0-9a-f-]{36}$/i.test(config.threadID)) throw new Error('接続先の識別子が不正です');
  if (statSync(config.secretFile).mode & 0o077) throw new Error('署名用ファイルは所有者だけが読み書きできる権限にしてください');
  const secret = readFileSync(config.secretFile, 'utf8').trim();
  if (secret.length < 32) throw new Error('署名用の秘密値が短すぎます');
  // OSが異常終了時にも解放するロックを取得してから、通知記録を読み込みます。
  const lock = new DatabaseSync(join(directory, 'receiver-lock.sqlite'));
  let journal;
  try { lock.exec('BEGIN EXCLUSIVE'); journal = new Journal(join(directory, 'deliveries.json')); }
  catch (error) { lock.close(); throw error; }
  let closing = false;
  let sending = null;
  let socket;
  let reconnect;
  let connectionTask;
  const activationTasks = new Set();
  const api = async args => (await execute(config.ghPath, ['api', ...args], { timeout: 30000, maxBuffer: 1024 * 1024 })).stdout;
  const authorize = async actor => {
    const result = JSON.parse(await api([`repos/${repository}/collaborators/${encodeURIComponent(actor)}/permission`]));
    return result.user?.permissions?.push === true || ['write', 'admin'].includes(result.permission);
  };
  const send = async message => {
    await execute(config.codexPath, ['queue', '--thread', config.threadID, '--message', message, '-C', root], { timeout: 60000, maxBuffer: 1024 * 1024 });
    console.log('Codexへ起動要求を送りました。実際の処理完了は確認記録で照合します。');
  };
  const timer = setInterval(async () => {
    if (sending || closing) return;
    sending = dispatch(journal, send, root, Date.now(), authorize);
    try { await sending; }
    catch { console.error('通知記録の処理に失敗しました。'); }
    finally { sending = null; }
  }, 5000);
  const connect = async () => {
    if (closing) return;
    try {
      // トークンは標準出力へ出さず、GitHubの認証付きWebSocket接続だけに渡します。
      const token = (await execute(config.ghPath, ['auth', 'token', '--hostname', 'github.com'], { timeout: 30000, maxBuffer: 16384 })).stdout.trim();
      if (closing) return;
      const current = new WebSocket(config.websocketURL, { headers: { Authorization: token }, maxPayload: 4 * 1024 * 1024, handshakeTimeout: 30000, followRedirects: false });
      socket = current;
      current.on('open', () => {
        if (closing || socket !== current) { current.close(); return; }
        const activationTask = (async () => {
          try {
            await api(['--method', 'PATCH', `repos/${repository}/hooks/${config.hookID}`, '-F', 'active=true']);
            if (!closing && socket === current) console.log('GitHubの認証付きWebhook受信を開始しました。');
          } catch { console.error('Webhookを有効化できませんでした。'); current.close(); }
        })();
        activationTasks.add(activationTask);
        activationTask.finally(() => activationTasks.delete(activationTask));
      });
      current.on('message', data => {
        if (closing || socket !== current) return;
        let status = 202;
        try {
          const packet = JSON.parse(data.toString());
          const headers = Object.fromEntries(Object.entries(packet.Header ?? packet.header ?? {}).map(([key, value]) => [key.toLowerCase(), Array.isArray(value) ? value[0] : value]));
          const raw = Buffer.from(packet.Body ?? packet.body ?? '', 'base64');
          const selected = selectEvent(raw, headers, secret, config.repositoryID);
          if (journal.receive(selected)) console.log(`Webhookを記録しました: ${selected.event} ${selected.delivery}`);
        } catch { status = 400; console.error('Webhookを検証・保存できませんでした。'); }
        if (current.readyState === WebSocket.OPEN) current.send(JSON.stringify({ Status: status, Header: { 'Content-Type': ['application/json'] }, Body: Buffer.from(JSON.stringify({ status })).toString('base64') }));
      });
      current.on('error', () => console.error('GitHub接続を再確認します。'));
      current.on('close', () => {
        if (socket === current) socket = null;
        if (!closing) reconnect = setTimeout(() => { connectionTask = connect(); }, 15000);
      });
    } catch {
      console.error('GitHubへの接続準備に失敗しました。');
      if (!closing) reconnect = setTimeout(() => { connectionTask = connect(); }, 15000);
    }
  };
  const stop = async () => {
    if (closing) return;
    closing = true;
    clearInterval(timer);clearTimeout(reconnect);
    if (connectionTask) await connectionTask.catch(() => {});
    await Promise.allSettled(activationTasks);
    if (socket && socket.readyState !== WebSocket.CLOSED) {
      const current = socket;
      await new Promise(done => { current.once('close', done); current.close(); });
    }
    if (sending) await sending.catch(() => {});
    lock.close();
  };
  process.once('SIGTERM', stop);process.once('SIGINT', stop);
  connectionTask = connect();
  await connectionTask;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  if (!process.argv[2]) throw new Error('設定ファイルの絶対パスを指定してください');
  main(process.argv[2]).catch(error => { console.error(error.message); process.exitCode = 1; });
}
