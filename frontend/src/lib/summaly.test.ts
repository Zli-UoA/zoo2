import { afterEach, expect, it, vi } from "vite-plus/test";
import { createSummalyClient, parseSummaly } from "./summaly";

afterEach(() => vi.restoreAllMocks());
it("maps Summaly fields and keeps empty metadata readable", () => {
  expect(
    parseSummaly(
      {
        title: "Title",
        description: "Text",
        thumbnail: "https://images.example/a.png",
        sitename: " Example Blog ",
        icon: "https://example.com/favicon.ico",
        player: { url: "javascript:alert(1)" },
      },
      "https://example.com",
    ),
  ).toEqual({
    title: "Title",
    description: "Text",
    image: "https://images.example/a.png",
    siteName: "Example Blog",
    icon: "https://example.com/favicon.ico",
  });
  expect(
    parseSummaly({ title: null, description: null, thumbnail: null }, "https://example.com"),
  ).toEqual({
    title: "example.com",
    description: "",
    image: null,
    siteName: "example.com",
    icon: null,
  });
  expect(parseSummaly({ sitename: "  " }, "https://example.com").siteName).toBe("example.com");
  expect(() => parseSummaly(null, "https://example.com")).toThrow();
});
it("requests the metadata service directly without app credentials", async () => {
  const fetcher = vi
    .spyOn(globalThis, "fetch")
    .mockResolvedValue(new Response(JSON.stringify({ title: "Title", thumbnail: null })));
  const signal = new AbortController().signal;
  const client = createSummalyClient("https://summaly.zliusercontent.com/url");
  await client.get("https://example.com/?a=1&b=2", { signal });
  const [url, options] = fetcher.mock.calls[0];
  const href = typeof url === "string" ? url : url instanceof URL ? url.href : url.url;
  expect(new URL(href).origin).toBe("https://summaly.zliusercontent.com");
  expect(new URL(href).searchParams.get("url")).toBe("https://example.com/?a=1&b=2");
  expect(options).toEqual({
    signal,
    credentials: "omit",
    referrerPolicy: "no-referrer",
    headers: { Accept: "application/json" },
  });
});
it("preserves retryable statuses and rejects unsafe endpoints", async () => {
  vi.spyOn(globalThis, "fetch").mockResolvedValue(new Response("busy", { status: 429 }));
  const options = { signal: new AbortController().signal };
  await expect(
    createSummalyClient("https://summaly.example/url").get("https://example.com", options),
  ).rejects.toMatchObject({ status: 429 });
  await expect(
    createSummalyClient("javascript:alert(1)").get("https://example.com", options),
  ).rejects.toMatchObject({ status: 400 });
});
