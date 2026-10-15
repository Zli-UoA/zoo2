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
});
