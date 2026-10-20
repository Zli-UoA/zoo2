import { describe, expect, it } from "vite-plus/test";

import { createI18n } from "../../lib/i18n";
import { OidcError } from "../../lib/oidc/client";
import { describeAuthError, formatAuthError } from "./authError";

describe("authentication error presentation", () => {
  it.each([
    [
      "ja",
      "state が一致しません（state_mismatch）",
      "ID トークンが期限切れです（id_token_expired）",
    ],
    [
      "en",
      "The authorization state does not match (state_mismatch)",
      "The ID token has expired (id_token_expired)",
    ],
    ["zh-Hant", "授權的 state 不符 (state_mismatch)", "ID 權杖已過期 (id_token_expired)"],
    ["zh-Hans", "授权的 state 不符 (state_mismatch)", "ID 令牌已过期 (id_token_expired)"],
    ["yue", "授權嘅 state 唔一致 (state_mismatch)", "ID 權杖過咗期 (id_token_expired)"],
  ] as const)("localizes client errors in %s", (locale, state, expired) => {
    const t = createI18n(locale).t;
    expect(
      formatAuthError(
        describeAuthError(new OidcError("state_mismatch", "state が一致しません")),
        t,
      ),
    ).toBe(state);
    expect(
      formatAuthError(
        describeAuthError(new OidcError("id_token_expired", "ID トークンが期限切れです")),
        t,
      ),
    ).toBe(expired);
  });

  it("keeps the error structured so changing language updates it without another login", async () => {
    const instance = createI18n("en");
    const error = describeAuthError(new OidcError("state_mismatch", "state が一致しません"));
    expect(formatAuthError(error, instance.t)).toBe(
      "The authorization state does not match (state_mismatch)",
    );
    await instance.changeLanguage("yue");
    expect(formatAuthError(error, instance.t)).toBe("授權嘅 state 唔一致 (state_mismatch)");
  });

  it("preserves provider descriptions, even when the provider code matches a client error", () => {
    const error = new OidcError("state_mismatch", "Custom provider diagnostic", {
      source: "provider",
    });
    expect(formatAuthError(describeAuthError(error), createI18n("en").t)).toBe(
      "Custom provider diagnostic (state_mismatch)",
    );
  });

  it("localizes locally generated HTTP diagnostics while retaining status and provider code", () => {
    const error = new OidcError("invalid_grant", "トークンエンドポイントが 400 を返しました", {
      reason: "token_request_failed",
      status: 400,
    });
    expect(formatAuthError(describeAuthError(error), createI18n("en").t)).toBe(
      "The token endpoint returned 400 (invalid_grant)",
    );
  });

  it("does not confuse an arbitrary message with the unauthorized-loop state", () => {
    const t = createI18n("en").t;
    expect(formatAuthError(describeAuthError(new Error("unauthorized-loop")), t)).toBe(
      "unauthorized-loop",
    );
    expect(formatAuthError({ kind: "unauthorizedLoop" }, t)).toContain(
      "API repeatedly returned 401",
    );
  });
});
