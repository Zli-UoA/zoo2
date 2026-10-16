import { describe, expect, it } from "vite-plus/test";

import { resolveConfig } from "../config";
import { buildShapeRequestUrl, resolveShapeUrl, shapeQueryParams } from "./query";

const directConfig = resolveConfig({ DEV: true, VITE_SHAPE_MODE: "direct" });
const proxyConfig = resolveConfig(
  { VITE_SHAPE_MODE: "proxy", VITE_API_BASE_URL: "http://localhost:8000" },
  {},
);

function paramsOf(url: string): URLSearchParams {
  return new URL(url).searchParams;
}

describe("shapeQueryParams", () => {
  it("uses the Electric subset where/params protocol in direct mode", () => {
    expect(shapeQueryParams("messages", directConfig, { channelId: "channel-1" })).toEqual({
      where: "channel_id = $1",
      "params[1]": "channel-1",
    });
    expect(shapeQueryParams("channelUsers", directConfig, { channelId: "channel-1" })).toEqual({
      where: "channel_id = $1",
      "params[1]": "channel-1",
    });
  });

  it("passes channel_id only in proxy mode (the proxy decides table/where)", () => {
    expect(shapeQueryParams("messages", proxyConfig, { channelId: "channel-1" })).toEqual({
      channel_id: "channel-1",
    });
    expect(shapeQueryParams("messages", proxyConfig)).toEqual({});
  });

  it("adds no filter for whole-table shapes", () => {
    expect(shapeQueryParams("channels", directConfig, { channelId: "ignored" })).toEqual({});
    expect(shapeQueryParams("users", proxyConfig, { channelId: "ignored" })).toEqual({});
  });
});

describe("buildShapeRequestUrl", () => {
  it("builds a direct Electric messages URL with table and where params", () => {
    const url = buildShapeRequestUrl("messages", directConfig, { channelId: "channel-1" });
    const params = paramsOf(url);
    expect(url.startsWith("http://localhost:3000/v1/shape?")).toBe(true);
    expect(params.get("table")).toBe("messages");
    expect(params.get("where")).toBe("channel_id = $1");
    expect(params.get("params[1]")).toBe("channel-1");
  });

  it("builds a direct Electric channels URL without a filter", () => {
    const params = paramsOf(buildShapeRequestUrl("channels", directConfig));
    expect(params.get("table")).toBe("channels");
    expect(params.get("where")).toBeNull();
  });

  it("builds a proxy messages URL with channel_id", () => {
    const url = buildShapeRequestUrl("messages", proxyConfig, { channelId: "channel-1" });
    expect(url).toBe("http://localhost:8000/api/shapes/messages?channel_id=channel-1");
    expect(paramsOf(url).get("where")).toBeNull();
  });

  it("builds a proxy channels URL without query parameters", () => {
    expect(buildShapeRequestUrl("channels", proxyConfig)).toBe(
      "http://localhost:8000/api/shapes/channels",
    );
  });
});

describe("resolveShapeUrl", () => {
  it("resolves same-origin proxy URLs against the page URL", () => {
    const config = resolveConfig({ VITE_SHAPE_MODE: "proxy", VITE_API_BASE_URL: "" });
    const url = buildShapeRequestUrl("messages", config, { channelId: "channel-1" });
    expect(resolveShapeUrl(url, "https://zoo.example/channels/channel-1")).toBe(
      "https://zoo.example/api/shapes/messages?channel_id=channel-1",
    );
  });

  it("keeps absolute URLs unchanged", () => {
    expect(resolveShapeUrl("http://localhost:8000/api/shapes/users", "https://zoo.example/")).toBe(
      "http://localhost:8000/api/shapes/users",
    );
  });
});
