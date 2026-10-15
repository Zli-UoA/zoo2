import { describe, expect, it } from "vite-plus/test";

import { DEFAULT_CONFIG, buildShapeUrl, joinUrl, resolveConfig } from "./config";

describe("resolveConfig", () => {
  it("defaults to direct mode against the local Electric endpoint", () => {
    const config = resolveConfig({});
    expect(config).toEqual(DEFAULT_CONFIG);
    expect(config.shapeMode).toBe("direct");
  });

  it("switches to proxy mode from the environment", () => {
    const config = resolveConfig({
      VITE_SHAPE_MODE: "proxy",
      VITE_API_BASE_URL: "http://localhost:8000",
    });
    expect(config.shapeMode).toBe("proxy");
    expect(config.apiBaseUrl).toBe("http://localhost:8000");
  });

  it("ignores an unknown shape mode", () => {
    expect(resolveConfig({ VITE_SHAPE_MODE: "bogus" }).shapeMode).toBe("direct");
  });

  it("reads the Electric URL, proxy path, and dev user from the environment", () => {
    const config = resolveConfig({
      VITE_ELECTRIC_SHAPE_URL: "http://electric.test:3000/v1/shape",
      VITE_SHAPE_PROXY_PATH: "/shapes",
      VITE_DEV_USER_ID: "dev-user-1",
    });
    expect(config.electricShapeUrl).toBe("http://electric.test:3000/v1/shape");
    expect(config.shapeProxyPath).toBe("/shapes");
    expect(config.devUserId).toBe("dev-user-1");
  });

  it("lets explicit overrides win over the environment", () => {
    const config = resolveConfig(
      { VITE_SHAPE_MODE: "direct" },
      { shapeMode: "proxy", apiBaseUrl: "" },
    );
    expect(config.shapeMode).toBe("proxy");
    expect(config.apiBaseUrl).toBe("");
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
    const config = resolveConfig({});
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
