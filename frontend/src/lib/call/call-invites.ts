/**
 * 通話の招待（呼び出し）。サーバー側の仕組みは docs/calls.md。
 *
 * 呼ばれた人・呼んだ人の `pending` の行は Electric の shape（`call_invites`）で届く。
 * 応答・拒否・取り消しはサーバーの API を呼ぶ。
 */
import { ApiError, type ApiClient } from "../api/client";
import type { CallInviteRow } from "../electric/types";

/** 招待を一覧にする。期限（`expires_at`）を過ぎたものは、サーバーが `missed` にするまでの間も除く。 */
export function liveCallInvites(rows: readonly CallInviteRow[], now: number): CallInviteRow[] {
  return rows
    .filter((row) => row.status === "pending" && callInviteExpiresAt(row) > now)
    .sort((a, b) => callInviteExpiresAt(a) - callInviteExpiresAt(b) || a.id.localeCompare(b.id));
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
): SplitCallInvites {
  const live = userId ? liveCallInvites(rows, now) : [];
  return {
    incoming: live.filter((row) => row.invitee_id === userId),
    outgoing: live.filter((row) => row.inviter_id === userId),
  };
}

/** 次に期限が来るまでのミリ秒（表示を更新する時刻の計算に使う）。招待がなければ null。 */
export function msUntilNextExpiry(rows: readonly CallInviteRow[], now: number): number | null {
  const times = rows
    .filter((row) => row.status === "pending")
    .map(callInviteExpiresAt)
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

/** Web Push の「応答」で開かれたときの URL の値（`?call_invite=ID&call_action=accept`）。 */
export interface CallInviteLaunch {
  inviteId: string;
  action: "accept" | null;
}

export function parseCallInviteLaunch(search: string): CallInviteLaunch | null {
  const params = new URLSearchParams(search);
  const inviteId = params.get("call_invite");
  if (!inviteId || !/^[0-9a-f-]{36}$/i.test(inviteId)) return null;
  return { inviteId, action: params.get("call_action") === "accept" ? "accept" : null };
}

/** 起動用のパラメーターを URL から外す（再読み込みで、もう一度応答しないように）。 */
export function stripCallInviteLaunch(href: string): string {
  const url = new URL(href);
  url.searchParams.delete("call_invite");
  url.searchParams.delete("call_action");
  return `${url.pathname}${url.search}${url.hash}`;
}
