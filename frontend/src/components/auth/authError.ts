import type { TFunction } from "i18next";

import { OidcError } from "../../lib/oidc/client";

export type AuthError =
  | { kind: "unauthorizedLoop" }
  | { kind: "oidc"; error: OidcError }
  | { kind: "message"; text: string };

/** 辞書のキーは `auth.oidcErrors.` + 接尾辞で組み立てる。x18n が前方一致で追えるようにする。 */
const ERROR_SUFFIXES = {
  discovery_failed: "discovery_failed",
  issuer_mismatch: "issuer_mismatch",
  invalid_discovery: "invalid_discovery",
  authorization_failed: "authorization_failed",
  invalid_callback: "invalid_callback",
  missing_pending_request: "missing_pending_request",
  state_mismatch: "state_mismatch",
  no_refresh_token: "no_refresh_token",
  session_reset: "session_reset",
  invalid_token_response: "invalid_token_response",
  token_request_failed: "token_request_failed",
  invalid_id_token: "invalid_id_token",
  id_token_issuer_mismatch: "id_token_issuer_mismatch",
  id_token_audience_mismatch: "id_token_audience_mismatch",
  id_token_expired: "id_token_expired",
  nonce_mismatch: "nonce_mismatch",
} as const;

export function describeAuthError(cause: unknown): AuthError {
  if (cause instanceof OidcError) return { kind: "oidc", error: cause };
  return { kind: "message", text: cause instanceof Error ? cause.message : String(cause) };
}

/** Translate at render time so an error follows language changes without retrying login. */
export function formatAuthError(error: AuthError | null, t: TFunction): string {
  if (!error) return "";
  if (error.kind === "unauthorizedLoop") return t("auth.unauthorizedLoop");
  if (error.kind === "message") return error.text;
  const cause = error.error;
  const suffix = Object.hasOwn(ERROR_SUFFIXES, cause.reason)
    ? ERROR_SUFFIXES[cause.reason as keyof typeof ERROR_SUFFIXES]
    : null;
  const message =
    cause.source === "client" && suffix
      ? t(`auth.oidcErrors.${suffix}`, { status: cause.status })
      : cause.message;
  return t("auth.errorWithCode", { message, code: cause.code });
}
