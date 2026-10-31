import { act, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vite-plus/test";
import { DEFAULT_CONFIG } from "../../../lib/config";
import { LinkPreviews } from "./LinkPreviews";

const config = { ...DEFAULT_CONFIG, summalyUrl: "", mediaProxyUrl: "" };
const api = vi.hoisted(() => ({ get: vi.fn() }));
vi.mock("../../../lib/summaly", () => ({ createSummalyClient: () => api }));
vi.mock("../../../lib/electric/collections-context", () => ({
  useOptionalCollections: () => ({ config }),
}));
afterEach(() => {
  vi.restoreAllMocks();
  vi.useRealTimers();
});

it("recovers mounted cards when the browser returns online after configuration failed", async () => {
  vi.useFakeTimers();
  const fetch = vi.spyOn(globalThis, "fetch").mockRejectedValue(new TypeError("offline"));
  api.get.mockResolvedValue({ title: "Online card", description: "", image: null });
  render(<LinkPreviews content="https://example.com/reconnected" />);
  await act(async () => {
    await vi.advanceTimersByTimeAsync(0);
  });
  expect(api.get).not.toHaveBeenCalled();
  fetch.mockImplementation(
    async (path) =>
      new Response(
        path === "/summaly-endpoint" ? "https://summaly.example/url" : "https://media.example/",
        { headers: { "Content-Type": "text/plain" } },
      ),
  );
  await act(async () => {
    window.dispatchEvent(new Event("online"));
    await vi.advanceTimersByTimeAsync(0);
  });
  expect(screen.getByRole("link", { name: /Online card/ })).toBeInTheDocument();
  expect(fetch).toHaveBeenCalledTimes(4);
});
