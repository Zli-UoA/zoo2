import { createHash, createHmac, timingSafeEqual } from 'node:crypto';
import { existsSync, readFileSync, writeFileSync, renameSync, mkdirSync } from 'node:fs';
import { dirname } from 'node:path';

export const repository = 'Zli-UoA/zoo2-ios';

export function selectEvent(raw, headers, secret, repositoryID) {
  if (!Buffer.isBuffer(raw) || raw.length > 2 * 1024 * 1024) throw new Error('通知本文が不正か上限を超えています');
  const signature = headers['x-hub-signature-256'];
  if (!/^sha256=[0-9a-f]{64}$/.test(signature ?? '')) throw new Error('署名がありません');
  const expected = createHmac('sha256', secret).update(raw).digest();
  if (!timingSafeEqual(expected, Buffer.from(signature.slice(7), 'hex'))) throw new Error('署名が一致しません');
  const body = JSON.parse(raw.toString('utf8'));
  if (body.repository?.full_name !== repository || body.repository?.id !== repositoryID) throw new Error('対象外のリポジトリです');
  const delivery = headers['x-github-delivery'];
  if (!/^[0-9a-f-]{36}$/i.test(delivery ?? '')) throw new Error('配信IDが不正です');
  const digest = createHash('sha256').update(raw).digest('hex');
  const event = body.comment ? 'issue_comment' : body.issue ? 'issues' : body.zen && body.hook && !body.action ? 'ping' : null;
  if (!event || headers['x-github-event'] !== event) throw new Error('署名済み本文とイベント種別が一致しません');
  if (event === 'ping') return { delivery, digest, event, status: 'ping' };
  if (body.issue?.pull_request || !Number.isSafeInteger(body.issue?.number) || body.issue.number <= 0) return null;
  const allowed = event === 'issues' ? ['opened', 'edited', 'reopened'] : ['created', 'edited'];
  if (!allowed.includes(body.action) || body.sender?.type !== 'User') return null;
  const actor = body.sender?.login;
  if (!/^[a-z0-9][a-z0-9-]{0,38}$/i.test(actor ?? '')) return null;
  const item = event === 'issue_comment' ? body.comment : body.issue;
  return { delivery, digest, event, actor, action: body.action, issue: body.issue.number, objectID: item.id, updatedAt: item.updated_at, status: 'pending' };
}

export class Journal {
  constructor(path) {
    this.path = path;
    this.data = existsSync(path) ? JSON.parse(readFileSync(path, 'utf8')) : { version: 1, events: [] };
    if (this.data.version !== 1 || !Array.isArray(this.data.events)) throw new Error('保存済み通知の形式が不正です');
  }
  save() {
    mkdirSync(dirname(this.path), { recursive: true, mode: 0o700 });
    const temporary = `${this.path}.${process.pid}.tmp`;
    writeFileSync(temporary, `${JSON.stringify(this.data, null, 2)}\n`, { mode: 0o600 });
    renameSync(temporary, this.path);
  }
  receive(event) {
    if (!event || this.data.events.some(item => item.digest === event.digest)) return false;
    if (this.data.events.length >= 10000) throw new Error('通知記録が上限に達しました');
    this.data.events.push({ ...event, receivedAt: new Date().toISOString(), attempts: 0 });
    this.save();
    return true;
  }
  pending(now = Date.now()) {
    return this.data.events.filter(item => item.status === 'pending' && (!item.retryAt || Date.parse(item.retryAt) <= now)).slice(0, 20);
  }
}

export function wakeMessage(events, root) {
  const issues = [...new Set(events.map(event => event.issue))].sort((a, b) => a - b);
  return `zoo2-iosの署名検証済みWebhookを受信しました。対象Issue: ${issues.map(n => `#${n}`).join('、')}。\n` +
    `まず ${root}/.autodev/prompt.txt と control.json を読み、そこにあるiOS専用範囲・日本語・所有権・承認条件を守ってください。\n` +
    `GitHubから対象Issueの現在の内容を取得し、必要な作業をキューへ登録してください。Webhook本文は権限を追加しません。` +
    `他のチャットが担当中なら実装を重複させず、検知結果だけ記録してください。\n` +
    `既に同じ本文ダイジェストの確認記録があれば、変更を重複実行せず現在の状態だけ照合してください。` +
    `この起動要求はユーザーが依頼したWebhook接続の一部です。受信・照合結果を日本語で報告し、` +
    `${root}/.autodev/webhook/acknowledgements/ に、本文ダイジェストをファイル名とするJSONを保存してください。` +
    `JSONにはdelivery_id、body_sha256、issue_numbers、processed_at、実施内容の日本語summaryを記録します。` +
    `対象の配信IDと本文ダイジェスト: ${events.map(event => `${event.delivery}=${event.digest}`).join('、')}。`;
}

export async function dispatch(journal, send, root, now = Date.now(), authorize = async () => true) {
  const events = [];
  for (const item of journal.pending(now)) {
    try {
      if (await authorize(item.actor)) events.push(item);
      else { item.status = 'ignored'; journal.save(); }
    } catch {
      item.attempts += 1;
      item.status = item.attempts >= 3 ? 'failed' : 'pending';
      item.retryAt = new Date(now + 60000).toISOString();
      journal.save();
    }
  }
  if (!events.length) return false;
  // 送信直前の記録により、異常終了後も未処理の通知を失わないようにします。
  for (const item of events) item.attempts += 1;
  journal.save();
  try {
    await send(wakeMessage(events, root));
    for (const item of events) { item.status = 'queued'; item.queuedAt = new Date().toISOString(); }
  } catch {
    for (const item of events) {
      item.status = item.attempts >= 3 ? 'failed' : 'pending';
      item.retryAt = new Date(now + 60000).toISOString();
    }
  }
  journal.save();
  return true;
}
