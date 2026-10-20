import type { TFunction } from "i18next";

import { OidcError } from "../../lib/oidc/client";

export type AuthError =
  | { kind: "unauthorizedLoop" }
  | { kind: "oidc"; error: OidcError }
  | { kind: "message"; text: string };

const ERROR_KEYS = {
  discovery_failed: "auth.oidcErrors.discovery_failed",
  issuer_mismatch: "auth.oidcErrors.issuer_mismatch",
  invalid_discovery: "auth.oidcErrors.invalid_discovery",
  authorization_failed: "auth.oidcErrors.authorization_failed",
  invalid_callback: "auth.oidcErrors.invalid_callback",
  missing_pending_request: "auth.oidcErrors.missing_pending_request",
  state_mismatch: "auth.oidcErrors.state_mismatch",
  no_refresh_token: "auth.oidcErrors.no_refresh_token",
  session_reset: "auth.oidcErrors.session_reset",
  invalid_token_response: "auth.oidcErrors.invalid_token_response",
  token_request_failed: "auth.oidcErrors.token_request_failed",
  invalid_id_token: "auth.oidcErrors.invalid_id_token",
  id_token_issuer_mismatch: "auth.oidcErrors.id_token_issuer_mismatch",
  id_token_audience_mismatch: "auth.oidcErrors.id_token_audience_mismatch",
  id_token_expired: "auth.oidcErrors.id_token_expired",
  nonce_mismatch: "auth.oidcErrors.nonce_mismatch",
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
  const key = Object.hasOwn(ERROR_KEYS, cause.reason)
    ? ERROR_KEYS[cause.reason as keyof typeof ERROR_KEYS]
    : null;
  const message =
    cause.source === "client" && key ? t(key, { status: cause.status }) : cause.message;
  return t("auth.errorWithCode", { message, code: cause.code });
}
