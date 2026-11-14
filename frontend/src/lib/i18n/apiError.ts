import type { TFunction } from "i18next";

import { ApiError } from "../api/client";
import { conversionBlockers } from "../api/encryptChannel";

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

/** E2EE などの API が返す `code`（`{"error", "code"}` の形）。なければ `null`。 */
function apiErrorCode(error: ApiError): string | null {
  const body = error.body;
  if (typeof body === "object" && body !== null && "code" in body) {
    const code = (body as { code: unknown }).code;
    return typeof code === "string" ? code : null;
  }
  return null;
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
    switch (apiErrorCode(cause)) {
      case "E2EE_DISABLED":
        return t("e2ee.errors.disabled");
      case "E2EE_PUBLIC_FORBIDDEN":
        return t("e2ee.errors.publicForbidden");
      case "E2EE_CONVERSION_BLOCKED": {
        const blockers = conversionBlockers(cause.body);
        if (blockers.length === 0) return t("common.errors.conflict");
        return blockers.map((blocker) => t(`e2ee.convert.blockers.${blocker}`)).join(" ");
      }
      case "E2EE_LEGACY_IMMUTABLE":
        return t("e2ee.legacy.editForbidden");
      case "E2EE_LIMIT_EXCEEDED":
        if ((cause.body as { limit?: unknown } | null)?.limit === "members") {
          return t("e2ee.errors.tooManyMembers");
        }
        break;
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
