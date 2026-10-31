import { act, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vite-plus/test";
import { DEFAULT_CONFIG } from "../../../lib/config";
import { LinkPreviews } from "./LinkPreviews.tsx";

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

it("shares configuration retries across cards and recovers when the browser returns online", async () => {
  vi.useFakeTimers();
  const fetch = vi.spyOn(globalThis, "fetch").mockRejectedValue(new TypeError("offline"));
  api.get.mockResolvedValue({ title: "Online card", description: "", image: null });
  const cards = Array.from({ length: 10 }, (_, i) => (
    <LinkPreviews key={i} content={`https://example.com/reconnected-${i}`} />
  ));
  const view = render(<>{cards}</>);
  await act(async () => {
    await vi.advanceTimersByTimeAsync(0);
  });
  expect(api.get).not.toHaveBeenCalled();
  expect(fetch).toHaveBeenCalledTimes(2);
  await act(async () => {
    await vi.advanceTimersByTimeAsync(2_000);
  });
  expect(fetch).toHaveBeenCalledTimes(4);
  view.unmount();
  await act(async () => {
    await vi.advanceTimersByTimeAsync(60_000);
    window.dispatchEvent(new Event("online"));
  });
  expect(fetch).toHaveBeenCalledTimes(4);
  render(<>{cards}</>);
  await act(async () => {
    await vi.advanceTimersByTimeAsync(0);
  });
  expect(fetch).toHaveBeenCalledTimes(6);
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
  expect(screen.getAllByRole("link", { name: /Online card/ })).toHaveLength(10);
  expect(fetch).toHaveBeenCalledTimes(8);
});
