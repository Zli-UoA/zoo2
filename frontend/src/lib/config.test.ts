import { describe, expect, it } from "vite-plus/test";

import {
  DEFAULT_CONFIG,
  TXID_MODES,
  buildShapeUrl,
  joinUrl,
  resolveConfig,
  resolveTxidMode,
} from "./config";

describe("resolveConfig", () => {
  it("enables the default preview user only with the explicit preview build flag", () => {
    expect(resolveConfig({ DEV: false, VITE_PREVIEW_AUTH: "true" }).devUserId).toBe(
      "00000000-0000-0000-0000-000000000001",
    );
    expect(resolveConfig({ DEV: false, VITE_PREVIEW_AUTH: "false" }).devUserId).toBeNull();
    expect(
      resolveConfig({
        VITE_PREVIEW_AUTH: "true",
        VITE_OIDC_ISSUER: "https://id.example",
        VITE_OIDC_CLIENT_ID: "zoo2",
      }).devUserId,
    ).toBeNull();
  });
  it("defaults to proxy mode in development when a dev user is configured", () => {
    const config = resolveConfig({ DEV: true, VITE_DEV_USER_ID: "dev-user-1" });
    expect(config).toEqual({ ...DEFAULT_CONFIG, shapeMode: "proxy", devUserId: "dev-user-1" });
  });

  it("defaults to proxy mode outside development", () => {
    expect(resolveConfig({ DEV: false }).shapeMode).toBe("proxy");
    expect(resolveConfig({}).shapeMode).toBe("proxy");
  });

  it("requires VITE_DEV_USER_ID for proxy mode in development", () => {
    expect(() => resolveConfig({ DEV: true })).toThrow(/VITE_DEV_USER_ID/);
    expect(() => resolveConfig({ DEV: true, VITE_SHAPE_MODE: "proxy" })).toThrow(
      /VITE_DEV_USER_ID/,
    );
  });

  it("switches to direct mode from the environment", () => {
    const config = resolveConfig({
      DEV: true,
      VITE_SHAPE_MODE: "direct",
      VITE_API_BASE_URL: "http://localhost:8000",
    });
    expect(config.shapeMode).toBe("direct");
    expect(config.apiBaseUrl).toBe("http://localhost:8000");
  });

  it("throws on an unrecognized shape mode instead of falling back", () => {
    expect(() => resolveConfig({ VITE_SHAPE_MODE: "bogus" })).toThrow(/VITE_SHAPE_MODE/);
  });

  it("reads the Electric URL and proxy path from the environment", () => {
    const config = resolveConfig({
      VITE_ELECTRIC_SHAPE_URL: "http://electric.test:3000/v1/shape",
      VITE_SHAPE_PROXY_PATH: "/shapes",
    });
    expect(config.electricShapeUrl).toBe("http://electric.test:3000/v1/shape");
    expect(config.shapeProxyPath).toBe("/shapes");
  });

  it("does not require a dev user in direct mode", () => {
    expect(resolveConfig({ DEV: true, VITE_SHAPE_MODE: "direct" }).devUserId).toBeNull();
  });

  it("only honors VITE_DEV_USER_ID in development", () => {
    expect(resolveConfig({ DEV: true, VITE_DEV_USER_ID: "dev-user-1" }).devUserId).toBe(
      "dev-user-1",
    );
    expect(resolveConfig({ DEV: false, VITE_DEV_USER_ID: "dev-user-1" }).devUserId).toBeNull();
    expect(resolveConfig({ VITE_DEV_USER_ID: "dev-user-1" }).devUserId).toBeNull();
  });

  it("lets explicit overrides win over the environment", () => {
    const config = resolveConfig(
      { DEV: true, VITE_SHAPE_MODE: "direct" },
      { shapeMode: "proxy", apiBaseUrl: "" },
    );
    expect(config.shapeMode).toBe("proxy");
    expect(config.apiBaseUrl).toBe("");
  });

  it("leaves OIDC unset so the dev X-Dev-User flow is unchanged", () => {
    expect(resolveConfig({ DEV: true, VITE_DEV_USER_ID: "dev-user-1" }).oidc).toBeNull();
    expect(DEFAULT_CONFIG.oidc).toBeNull();
  });

  it("reads OIDC settings from the environment", () => {
    const config = resolveConfig({
      VITE_OIDC_ISSUER: "https://id.example/oauth2/openid/zoo",
      VITE_OIDC_CLIENT_ID: "zoo",
    });
    expect(config.oidc).toEqual({
      issuer: "https://id.example/oauth2/openid/zoo",
      clientId: "zoo",
      redirectUri: null,
      scope: "openid profile email",
      postLogoutRedirectUri: null,
    });
  });

  it("overrides scope, redirect, and post-logout redirect", () => {
    const config = resolveConfig({
      VITE_OIDC_ISSUER: "https://id.example/oauth2/openid/zoo/",
      VITE_OIDC_CLIENT_ID: "zoo",
      VITE_OIDC_REDIRECT_URI: "https://app.example/auth/callback",
      VITE_OIDC_SCOPE: "openid profile offline_access",
      VITE_OIDC_POST_LOGOUT_REDIRECT_URI: "https://app.example/logged-out",
    });
    expect(config.oidc).toMatchObject({
      redirectUri: "https://app.example/auth/callback",
      scope: "openid profile offline_access",
      postLogoutRedirectUri: "https://app.example/logged-out",
    });
  });

  it("requires issuer and client ID together", () => {
    expect(() => resolveConfig({ VITE_OIDC_ISSUER: "https://id.example" })).toThrow(
      /VITE_OIDC_ISSUER and VITE_OIDC_CLIENT_ID/,
    );
    expect(() => resolveConfig({ VITE_OIDC_CLIENT_ID: "zoo" })).toThrow(
      /VITE_OIDC_ISSUER and VITE_OIDC_CLIENT_ID/,
    );
  });

  it("does not require a dev user once OIDC is configured", () => {
    const config = resolveConfig({
      DEV: true,
      VITE_OIDC_ISSUER: "https://id.example/oauth2/openid/zoo",
      VITE_OIDC_CLIENT_ID: "zoo",
      VITE_DEV_USER_ID: "dev-user-1",
    });
    expect(config.devUserId).toBeNull();
  });
});

describe("resolveTxidMode", () => {
  it("defaults to awaitTxId", () => {
    expect(DEFAULT_CONFIG.txidMode).toBe("awaitTxId");
    expect(resolveConfig({ DEV: true, VITE_DEV_USER_ID: "dev-user-1" }).txidMode).toBe("awaitTxId");
  });

  it("reads a supported mode from the environment", () => {
    expect(TXID_MODES).toEqual(["awaitTxId", "awaitMatch", "skip"]);
    expect(resolveConfig({ VITE_TXID_MODE: "awaitMatch" }).txidMode).toBe("awaitMatch");
    expect(resolveConfig({ VITE_TXID_MODE: "skip" }).txidMode).toBe("skip");
  });

  it("throws on an unrecognized txid mode instead of falling back", () => {
    expect(() => resolveTxidMode("bogus")).toThrow(/VITE_TXID_MODE/);
    expect(() => resolveConfig({ VITE_TXID_MODE: "bogus" })).toThrow(/VITE_TXID_MODE/);
  });

  it("lets explicit overrides win over the environment", () => {
    expect(resolveConfig({ VITE_TXID_MODE: "skip" }, { txidMode: "awaitTxId" }).txidMode).toBe(
      "awaitTxId",
    );
  });
});

describe("joinUrl", () => {
  it("joins without duplicating slashes", () => {
    expect(joinUrl("http://localhost:8000/", "/api/messages")).toBe(
      "http://localhost:8000/api/messages",
    );
  });

  it("keeps the path relative when the base is empty", () => {
    expect(joinUrl("", "/api/messages")).toBe("/api/messages");
  });
});

describe("buildShapeUrl", () => {
  it("builds direct Electric URLs with the table parameter", () => {
    const config = resolveConfig({ DEV: true, VITE_SHAPE_MODE: "direct" });
    expect(buildShapeUrl("messages", config, { channel_id: "channel-1" })).toBe(
      "http://localhost:3000/v1/shape?table=messages&channel_id=channel-1",
    );
  });

  it("builds proxy URLs under /api/shapes with the proxy path", () => {
    const config = resolveConfig({
      VITE_SHAPE_MODE: "proxy",
      VITE_API_BASE_URL: "http://localhost:8000",
    });
    expect(buildShapeUrl("channelUsers", config)).toBe(
      "http://localhost:8000/api/shapes/channel_users",
    );
    expect(buildShapeUrl("channels", config)).toBe("http://localhost:8000/api/shapes/channels");
  });

  it("keeps proxy URLs relative when no API base URL is configured", () => {
    const config = resolveConfig({ VITE_SHAPE_MODE: "proxy" }, { apiBaseUrl: "" });
    expect(buildShapeUrl("users", config)).toBe("/api/shapes/users");
  });

  it("appends protocol parameters in proxy mode", () => {
    const config = resolveConfig({ VITE_SHAPE_MODE: "proxy" }, { apiBaseUrl: "" });
    expect(buildShapeUrl("messages", config, { offset: -1, live: "true" })).toBe(
      "/api/shapes/messages?offset=-1&live=true",
    );
  });
});
