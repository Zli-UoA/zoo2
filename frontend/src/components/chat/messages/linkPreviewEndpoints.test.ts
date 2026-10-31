import { afterEach, expect, it, vi } from "vite-plus/test";
import { DEFAULT_CONFIG } from "../../../lib/config";

afterEach(() => {
  vi.unstubAllGlobals();
  vi.resetModules();
});

it("retries runtime configuration after a network failure and then shares success", async () => {
  const { previewEndpoints } = await import("./linkPreviewEndpoints");
  const fetch = vi.fn().mockRejectedValue(new TypeError("offline"));
  vi.stubGlobal("fetch", fetch);
  await expect(previewEndpoints(DEFAULT_CONFIG)).rejects.toThrow("offline");
  fetch.mockImplementation(
    async (path: string) =>
      new Response(
        path === "/summaly-endpoint" ? "https://summaly.example/url" : "https://media.example/",
        { headers: { "Content-Type": "text/plain" } },
      ),
  );
  const expected = { summaly: "https://summaly.example/url", media: "https://media.example/" };
  expect(await previewEndpoints(DEFAULT_CONFIG)).toEqual(expected);
  expect(await previewEndpoints(DEFAULT_CONFIG)).toEqual(expected);
  expect(fetch).toHaveBeenCalledTimes(4);
});

it("does not retain HTTP failures as empty configuration", async () => {
  const { previewEndpoints } = await import("./linkPreviewEndpoints");
  const fetch = vi.fn().mockImplementation(async () => new Response("error", { status: 503 }));
  vi.stubGlobal("fetch", fetch);
  await expect(previewEndpoints(DEFAULT_CONFIG)).rejects.toThrow();
  fetch.mockImplementation(
    async () => new Response("", { headers: { "Content-Type": "text/plain" } }),
  );
  expect(await previewEndpoints(DEFAULT_CONFIG)).toEqual({ summaly: "", media: "" });
  expect(fetch).toHaveBeenCalledTimes(4);
});
