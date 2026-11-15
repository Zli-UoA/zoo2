/**
 * 通話の招待（呼び出し）。サーバー側の仕組みは docs/calls.md。
 *
 * 呼ばれた人・呼んだ人の `pending` の行は Electric の shape（`call_invites`）で届く。
 * 応答・拒否・取り消しはサーバーの API を呼ぶ。
 */
import { ApiError, type ApiClient } from "../api/client";
import type { CallInviteRow } from "../electric/types";

/**
 * 招待を一覧にする。期限を過ぎたものは、サーバーが `missed` にするまでの間も除く。
 * 期限の時刻は、端末の時計の進み遅れに左右されないよう、`deadlineOf`（既定はサーバーの `expires_at`）で決める。
 */
export function liveCallInvites(
  rows: readonly CallInviteRow[],
  now: number,
  deadlineOf: (row: CallInviteRow) => number = callInviteExpiresAt,
): CallInviteRow[] {
  return rows
    .filter((row) => row.status === "pending" && deadlineOf(row) > now)
    .sort((a, b) => deadlineOf(a) - deadlineOf(b) || a.id.localeCompare(b.id));
}

/** 招待の有効な長さ（ミリ秒）。サーバーの `created_at` と `expires_at` の差で、端末の時計を使わない。 */
export function callInviteDurationMs(
  row: Pick<CallInviteRow, "created_at" | "expires_at">,
): number {
  const created = callInviteExpiresAt({ expires_at: row.created_at });
  const expires = callInviteExpiresAt(row);
  return created > 0 && expires > created ? expires - created : 0;
}

/** Electric の `timestamptz`（`2026-11-14 10:00:00.123+00`）をミリ秒にする。読めなければ 0（期限切れ）。 */
export function callInviteExpiresAt(row: Pick<CallInviteRow, "expires_at">): number {
  let value = row.expires_at.trim().replace(" ", "T");
  value = value.replace(/([+-]\d{2})$/, "$1:00");
  const time = Date.parse(value);
  return Number.isNaN(time) ? 0 : time;
}

export interface SplitCallInvites {
  /** 自分が呼ばれている招待。期限の早い順。 */
  incoming: CallInviteRow[];
  /** 自分が呼んでいる招待。 */
  outgoing: CallInviteRow[];
}

export function splitCallInvites(
  rows: readonly CallInviteRow[],
  userId: string | null,
  now: number,
  deadlineOf?: (row: CallInviteRow) => number,
): SplitCallInvites {
  const live = userId ? liveCallInvites(rows, now, deadlineOf) : [];
  return {
    incoming: live.filter((row) => row.invitee_id === userId),
    outgoing: live.filter((row) => row.inviter_id === userId),
  };
}

/** 次に期限が来るまでのミリ秒（表示を更新する時刻の計算に使う）。招待がなければ null。 */
export function msUntilNextExpiry(
  rows: readonly CallInviteRow[],
  now: number,
  deadlineOf: (row: CallInviteRow) => number = callInviteExpiresAt,
): number | null {
  const times = rows
    .filter((row) => row.status === "pending")
    .map(deadlineOf)
    .filter((time) => time > now);
  return times.length > 0 ? Math.min(...times) - now : null;
}

interface InviteResponse {
  ok: boolean;
  channel_id: string;
}

type Api = Pick<ApiClient, "request">;

/** 応答する。通話への参加は、続けて `joinCall` で行う。すでに終わっていれば `null`（409・404）。 */
export async function acceptCallInvite(api: Api, inviteId: string): Promise<string | null> {
  try {
    const res = await api.request<InviteResponse>(`/api/call-invites/${inviteId}/accept`, {
      method: "POST",
    });
    return res.channel_id;
  } catch (err) {
    if (isGone(err)) return null;
    throw err;
  }
}

/** 拒否する。すでに終わっていても、成功と同じに扱う。 */
export async function declineCallInvite(api: Api, inviteId: string): Promise<void> {
  await endInvite(api, inviteId, "decline");
}

/** 呼び出しをやめる（呼んだ人）。すでに終わっていても、成功と同じに扱う。 */
export async function cancelCallInvite(api: Api, inviteId: string): Promise<void> {
  await endInvite(api, inviteId, "cancel");
}

async function endInvite(api: Api, inviteId: string, action: "decline" | "cancel") {
  try {
    await api.request(`/api/call-invites/${inviteId}/${action}`, { method: "POST" });
  } catch (err) {
    if (!isGone(err)) throw err;
  }
}

/** 通話中に、チャンネルのメンバーを呼ぶ。 */
export async function inviteToCall(
  api: Api,
  channelId: string,
  userIds: readonly string[],
): Promise<void> {
  await api.request(`/api/channels/${channelId}/call/invites`, {
    method: "POST",
    body: { user_ids: userIds },
  });
}

function isGone(err: unknown): boolean {
  const status = err instanceof ApiError ? err.status : (err as { status?: number })?.status;
  return status === 409 || status === 404;
}

/**
 * Web Push の「応答」を押したことの印を入れる Cache（`public/sw.js` が書き、画面が一度だけ読んで消す）。
 *
 * URL（`?call_invite=ID`）は、どの招待かの手がかりにすぎない。URL だけで自動応答すると、
 * リンクを開かせるだけで通話に入れてしまうので、応答するのは、この印があるときだけにする。
 */
export const CALL_ACCEPT_CACHE = "zoo-call-accept";
export const CALL_ACCEPT_MARK_TTL_MS = 60_000;

/** 通知のボタン。印は、押したボタンごとに別に残す。 */
export type CallMarkKind = "accept" | "decline";

export function callMarkPath(kind: CallMarkKind, inviteId: string): string {
  return `/__call-${kind}/${inviteId}`;
}

export function callAcceptMarkPath(inviteId: string): string {
  return callMarkPath("accept", inviteId);
}

/** 印があれば、消して `true` を返す（複数のタブのうち、先に読んだ 1 つだけが `true`）。 */
export async function consumeCallMark(
  kind: CallMarkKind,
  inviteId: string,
  now: number = Date.now(),
): Promise<boolean> {
  try {
    const cache = await caches.open(CALL_ACCEPT_CACHE);
    const path = callMarkPath(kind, inviteId);
    const response = await cache.match(path);
    if (!response) return false;
    const removed = await cache.delete(path);
    const markedAt = Number(await response.text());
    return removed && Number.isFinite(markedAt) && now - markedAt < CALL_ACCEPT_MARK_TTL_MS;
  } catch {
    return false;
  }
}

/** 「応答」の印を、一度だけ読む。 */
export function consumeCallAcceptMark(inviteId: string, now?: number): Promise<boolean> {
  return consumeCallMark("accept", inviteId, now);
}

/**
 * 応答を画面に頼むイベント。デスクトップ版（Tauri）が、トーストの「応答」の印を Rust 側から
 * 取れたときに出す（`DesktopBridge`）。`IncomingCallDialog` が受けて、応答して通話に入る。
 */
export const CALL_ACCEPT_REQUEST_EVENT = "zoo:call-invite-accept";

export function requestAcceptCallInvite(inviteId: string): void {
  window.dispatchEvent(new CustomEvent<string>(CALL_ACCEPT_REQUEST_EVENT, { detail: inviteId }));
}

/** 出ている着信の通知（Web Push）を閉じる。応答・拒否のあと、他の端末の通知は `call_invite_end` が閉じる。 */
export async function closeCallInviteNotification(inviteId: string): Promise<void> {
  try {
    const registration = await navigator.serviceWorker?.getRegistration();
    const open = (await registration?.getNotifications({ tag: `zoo-call-${inviteId}` })) ?? [];
    for (const notification of open) notification.close();
  } catch {
    // 通知を閉じられなくても、通話には影響しない。
  }
}

/** Web Push の通知から開かれたときの URL の値（`?call_invite=ID`）。招待の ID だけを読む。 */
export function parseCallInviteLaunch(search: string): string | null {
  const inviteId = new URLSearchParams(search).get("call_invite");
  return inviteId && /^[0-9a-f-]{36}$/i.test(inviteId) ? inviteId : null;
}

/** 起動用のパラメーターを URL から外す（再読み込みで、もう一度処理しないように）。 */
export function stripCallInviteLaunch(href: string): string {
  const url = new URL(href);
  url.searchParams.delete("call_invite");
  return `${url.pathname}${url.search}${url.hash}`;
}
