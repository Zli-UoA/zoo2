import { buildShapeUrl, type AppConfig } from "../config";
import type { ShapeName } from "./schema";

export interface ShapeQueryOptions {
  /** メッセージ / メンバー一覧を絞り込むチャンネル ID。 */
  channelId?: string;
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
 * いずれも `deleted_at` の除外はクエリ側（live query）で行う。
 */
export function shapeQueryParams(
  shape: ShapeName,
  config: AppConfig,
  options: ShapeQueryOptions = {},
): ShapeQueryParams {
  if (config.shapeMode === "proxy") {
    if ((shape === "messages" || shape === "channelUsers") && options.channelId) {
      return { channel_id: options.channelId };
    }
    return {};
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
