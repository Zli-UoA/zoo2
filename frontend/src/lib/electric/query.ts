import { buildShapeUrl, type AppConfig } from "../config";
import type { ShapeName } from "./schema";

export interface ShapeQueryOptions {
  /** メッセージ / リアクション / メンバー一覧を絞り込むチャンネル ID。 */
  channelId?: string;
  userId?: string;
}

export type ShapeQueryParams = Record<string, string | number>;

/**
 * Shape ごとの追加クエリパラメータをモード別に組み立てる。
 *
 * - direct（Electric 直結）: Electric の subset プロトコルに合わせて
 *   `where` と位置パラメータ `params[1]` を渡す（table は `buildShapeUrl()` が付与する）。
 * - proxy（バックエンド経由）: プロキシが `table` / `where` を確定させるため、
 *   ルーティングに必要な `channel_id` だけを渡す。
 *
 * なお `channelUsers` を `channelId` 付きで呼ぶ経路は現状未使用（コレクションは
 * 全件購読）。チャンネル単位で購読する場合に使えるよう汎用化してある。
 *
 * いずれも `deleted_at` の除外はクエリ側（live query）で行う。
 */
export function shapeQueryParams(
  shape: ShapeName,
  config: AppConfig,
  options: ShapeQueryOptions = {},
): ShapeQueryParams {
  if (config.shapeMode === "proxy") {
    if (
      (shape === "messages" ||
        shape === "messageReactions" ||
        shape === "channelUsers" ||
        shape === "attachments") &&
      options.channelId
    ) {
      return { channel_id: options.channelId };
    }
    return {};
  }

  if (shape === "notifications") {
    if (!options.userId) throw new Error("Notification shapes require a user ID");
    return { where: "user_id = $1", "params[1]": options.userId };
  }
  if (shape === "messageReactions" && options.channelId) {
    return {
      where: "message_id IN (SELECT id FROM messages WHERE channel_id = $1 AND deleted_at IS NULL)",
      "params[1]": options.channelId,
    };
  }
  if (shape === "attachments" && options.channelId) {
    return {
      where: "channel_id = $1 AND message_id IS NOT NULL",
      "params[1]": options.channelId,
    };
  }
  if ((shape === "messages" || shape === "channelUsers") && options.channelId) {
    return { where: "channel_id = $1", "params[1]": options.channelId };
  }
  return {};
}

/** Shape の購読 URL を、モードに応じたクエリパラメータ付きで生成する。 */
export function buildShapeRequestUrl(
  shape: ShapeName,
  config: AppConfig,
  options: ShapeQueryOptions = {},
): string {
  return buildShapeUrl(shape, config, shapeQueryParams(shape, config, options));
}

/**
 * Electric の `ShapeStream` は購読 URL を base なしの `new URL()` で解釈するため、
 * 相対 URL（同一オリジン構成で `VITE_API_BASE_URL` が空のとき）を渡すと
 * `Invalid URL` で失敗する。ページの URL を基準に絶対 URL へ解決してから渡す。
 */
export function resolveShapeUrl(url: string, baseHref: string): string {
  return new URL(url, baseHref).toString();
}
