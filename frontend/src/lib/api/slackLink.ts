import type { ApiClient } from "./client";

export interface SlackLink {
  team_id: string;
  team_name: string;
  slack_user_id: string;
  slack_user_name: string;
  linked_at: string;
}

export interface SlackLinkState {
  /** サーバーに Slack アプリ (Client ID など) が設定されているか */
  configured: boolean;
  link: SlackLink | null;
}

export function getSlackLink(client: ApiClient, signal?: AbortSignal): Promise<SlackLinkState> {
  return client.get<SlackLinkState>("/api/slack/link", { signal });
}

/**
 * 認可 URL を受け取る。Bearer トークンで認証するため、ブラウザを直接
 * バックエンドへ遷移させず、受け取った Slack の URL へ移動する。
 */
export function startSlackLink(client: ApiClient): Promise<{ authorize_url: string }> {
  return client.request<{ authorize_url: string }>("/api/slack/oauth/start", { method: "POST" });
}

export function unlinkSlack(client: ApiClient): Promise<SlackLinkState> {
  return client.request<SlackLinkState>("/api/slack/link", { method: "DELETE" });
}

export interface SlackManifest {
  configured: boolean;
  manifest: Record<string, unknown>;
}

export function getSlackManifest(client: ApiClient, signal?: AbortSignal): Promise<SlackManifest> {
  return client.get<SlackManifest>("/api/admin/slack/manifest", { signal });
}

/** ログイン中のユーザーとして連携を完了する。state を作ったユーザー本人でなければ拒否される。 */
export function completeSlackLink(
  client: ApiClient,
  code: string,
  state: string,
): Promise<SlackLinkState> {
  return client.request<SlackLinkState>("/api/slack/oauth/complete", {
    method: "POST",
    body: { code, state },
  });
}

/** Slack からの戻り (`?code=...&state=...` または `?slack=error&reason=...`) */
export type SlackLinkResult =
  | { status: "authorized"; code: string; state: string }
  | { status: "error"; reason: string };

export function readSlackLinkResult(search: Record<string, unknown>): SlackLinkResult | null {
  if (typeof search.code === "string" && typeof search.state === "string") {
    return { status: "authorized", code: search.code, state: search.state };
  }
  if (search.slack === "error") {
    return { status: "error", reason: typeof search.reason === "string" ? search.reason : "" };
  }
  return null;
}

/** `complete` の失敗理由。サーバーの HTTP ステータスに対応する */
export function slackLinkErrorReason(status: number | null): string {
  switch (status) {
    case 400:
      return "invalid_state";
    case 403:
      return "team_mismatch";
    case 409:
      return "already_linked";
    default:
      return "failed";
  }
}
