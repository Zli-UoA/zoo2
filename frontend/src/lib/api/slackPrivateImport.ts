import type { ApiClient } from "./client";

/**
 * 連携した Slack アカウントで、自分が作った非公開チャンネルを取り込む (`/api/slack/private-channels`)。
 * エクスポートの ZIP は使わず、本人のトークンで Slack の API を読む。履歴は、バックエンドの同期が全履歴から取る。
 */
export type PrivateChannelStatus = "new" | "queued" | "running" | "done" | "error" | "hidden";

export interface PrivateSlackChannel {
  slack_channel_id: string;
  name: string;
  is_archived: boolean;
  members: number;
  status: PrivateChannelStatus;
  /** `new` のときだけ。取り込み先の名前の提案 (使われている名前なら `slack-` を付ける) */
  suggested_name?: string;
  name_taken?: boolean;
  /** 取り込み済みのとき。参加していない人には `hidden` で、名前も ID も返らない */
  channel_id?: string;
  channel_name?: string;
}

/** 一覧を返せない理由 (`not_linked`・`team_mismatch`・`missing_scope`・`token_revoked` など)。連携し直すと直るものがある */
export type PrivateChannelsUnavailable = string;

export interface PrivateSlackChannels {
  linked: boolean;
  reason?: PrivateChannelsUnavailable;
  channels: PrivateSlackChannel[];
}

export interface PrivateChannelPick {
  slack_channel_id: string;
  name: string;
}

/** 取り込みの進み具合を、画面でポーリングするか。`error` も、バックエンドが自動で再試行するので続ける */
export function hasActivePrivateImport(channels: PrivateSlackChannel[]): boolean {
  return channels.some(
    (channel) =>
      channel.status === "queued" || channel.status === "running" || channel.status === "error",
  );
}

/** 連携し直すと使えるようになる理由か (トークンの失効・スコープ不足) */
export function needsRelink(reason: string | undefined): boolean {
  return (
    reason === "missing_scope" ||
    reason === "token_revoked" ||
    reason === "token_expired" ||
    reason === "invalid_auth" ||
    reason === "not_authed" ||
    reason === "account_inactive"
  );
}

export function getPrivateSlackChannels(
  client: ApiClient,
  signal?: AbortSignal,
): Promise<PrivateSlackChannels> {
  return client.get<PrivateSlackChannels>("/api/slack/private-channels", { signal });
}

/** 1 回の要求で取り込むのは 1 チャンネル */
export function startPrivateSlackImport(
  client: ApiClient,
  pick: PrivateChannelPick,
): Promise<{ slack_channel_id: string; channel_id: string; channel_name: string }> {
  return client.request("/api/slack/private-channels/import", {
    method: "POST",
    body: pick,
  });
}
