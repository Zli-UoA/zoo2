import type { TFunction } from "i18next";

import { ApiError } from "../api/client";

/**
 * チャンネル名の一意制約に当たった書き込みか。
 *
 * 作成（POST /api/channels）と改名（PATCH /api/channels/{id}）の 409 は
 * 名前の重複を意味する。参加・退出など別操作の 409 と区別するため、
 * パスとメソッドまで確認する。
 */
function isChannelNameConflict(error: ApiError): boolean {
  if (error.status !== 409 || !error.path) {
    return false;
  }
  if (error.method === "POST" && error.path === "/api/channels") {
    return true;
  }
  return error.method === "PATCH" && /^\/api\/channels\/[^/]+$/.test(error.path);
}

/**
 * `ApiError` を現在の言語のユーザー向け文言にする。
 *
 * バックエンドの `error` は英語のため、そのまま画面に出さない。ステータスと
 * 操作（パス・メソッド）から既知の翻訳を選び、判別できない失敗も汎用文言に
 * 置き換える。401 は認証層が再ログインへ導くが、文言自体はここで用意する。
 */
export function localizeApiError(cause: unknown, t: TFunction): string {
  if (cause instanceof ApiError) {
    if (isChannelNameConflict(cause)) {
      return t("channels.duplicateName");
    }
    switch (cause.status) {
      case 400:
        return t("common.errors.badRequest");
      case 401:
        return t("common.errors.unauthorized");
      case 403:
        return t("common.errors.forbidden");
      case 404:
        return t("common.errors.notFound");
      case 409:
        return t("common.errors.conflict");
      case 429:
        return t("common.errors.rateLimited");
      default:
        return cause.status >= 500 ? t("common.errors.server") : t("common.errors.unexpected");
    }
  }
  if (cause instanceof TypeError) {
    return t("common.errors.network");
  }
  return t("common.errors.unexpected");
}
