import { afterEach, expect, it, vi } from "vite-plus/test";
import { ApiError } from "../../../lib/api/client";
import { acquirePreview } from "./linkPreviewRequests";
import type { SummalyClient } from "../../../lib/summaly";

afterEach(() => vi.useRealTimers());

it("shares requests and aborts only when the final consumer leaves", async () => {
  let signal!: AbortSignal;
  const client = {
    get: vi.fn<SummalyClient["get"]>((_url, options) => {
      signal = options.signal;
      return new Promise((_resolve, reject) => {
        signal.addEventListener("abort", () => reject(signal.reason), { once: true });
      });
    }),
  };
  const a = acquirePreview(client, "shared", "https://example.com/shared");
  const b = acquirePreview(client, "shared", "https://example.com/shared");
  await vi.waitFor(() => expect(client.get).toHaveBeenCalledTimes(1));
  a.release();
  expect(signal.aborted).toBe(false);
  b.release();
  expect(signal.aborted).toBe(true);
  expect(await b.request).toBeNull();
});

it("removes abandoned queued URLs so the new view gets the next slot", async () => {
  const client = {
    get: vi.fn<SummalyClient["get"]>(
      (_url, { signal }) =>
        new Promise((_resolve, reject) => {
          signal.addEventListener("abort", () => reject(signal.reason), { once: true });
        }),
    ),
  };
  const old = Array.from({ length: 8 }, (_, i) =>
    acquirePreview(client, `old-${i}`, `https://example.com/old-${i}`),
  );
  await vi.waitFor(() => expect(client.get).toHaveBeenCalledTimes(4));
  // Cancel waiting consumers first, then release an active slot.
  old.slice(4).forEach((work) => work.release());
  const current = acquirePreview(client, "current", "https://example.com/current");
  old[0]!.release();
  await vi.waitFor(() => expect(client.get).toHaveBeenCalledTimes(5));
  expect(vi.mocked(client.get).mock.calls[4]?.[0]).toBe("https://example.com/current");
  old.forEach((work) => work.release());
  current.release();
  await Promise.all([...old.map((work) => work.request), current.request]);
});

it("retries upstream 422 failures and does not cache an exhausted failure", async () => {
  vi.useFakeTimers();
  const client = { get: vi.fn().mockRejectedValue(new ApiError(422, "error")) };
  const first = acquirePreview(client, "422", "https://example.com/temporary");
  await vi.advanceTimersByTimeAsync(14_000);
  expect(client.get).toHaveBeenCalledTimes(4);
  expect(await first.request).toBeNull();
  first.release();
  client.get.mockResolvedValue({ title: "Recovered", description: "", image: null });
  const second = acquirePreview(client, "422", "https://example.com/temporary");
  expect(await second.request).toMatchObject({ title: "Recovered" });
  expect(client.get).toHaveBeenCalledTimes(5);
  second.release();
});

it("cancels retry backoff when the final consumer leaves", async () => {
  vi.useFakeTimers();
  const client = { get: vi.fn().mockRejectedValue(new ApiError(422, "error")) };
  const work = acquirePreview(client, "backoff", "https://example.com/backoff");
  await vi.advanceTimersByTimeAsync(0);
  work.release();
  await vi.advanceTimersByTimeAsync(60_000);
  expect(client.get).toHaveBeenCalledTimes(1);
  expect(await work.request).toBeNull();
});

it("serves stale metadata immediately and shares a refresh that updates all subscribers", async () => {
  vi.useFakeTimers();
  const { cachedPreview, subscribePreview } = await import("./linkPreviewRequests");
  const old = { title: "Cached", description: "", image: null };
  let finish!: (data: typeof old) => void;
  const client = {
    get: vi
      .fn()
      .mockResolvedValueOnce(old)
      .mockImplementation(
        () =>
          new Promise((resolve) => {
            finish = resolve;
          }),
      ),
  };
  const first = acquirePreview(client, "stale-shared", "https://example.com/stale-shared");
  await first.request;
  first.release();
  await vi.advanceTimersByTimeAsync(300_001);
  expect(cachedPreview("stale-shared")).toBe(old);
  const a = vi.fn(),
    b = vi.fn();
  const unsubA = subscribePreview("stale-shared", a),
    unsubB = subscribePreview("stale-shared", b);
  const refresh = acquirePreview(client, "stale-shared", "https://example.com/stale-shared");
  const shared = acquirePreview(client, "stale-shared", "https://example.com/stale-shared");
  await vi.advanceTimersByTimeAsync(0);
  expect(client.get).toHaveBeenCalledTimes(2);
  expect(cachedPreview("stale-shared")).toBe(old);
  const next = { ...old, title: "Updated" };
  finish(next);
  expect(await refresh.request).toBe(next);
  expect(await shared.request).toBe(next);
  expect(cachedPreview("stale-shared")).toBe(next);
  expect(a).toHaveBeenCalledWith(next);
  expect(b).toHaveBeenCalledWith(next);
  refresh.release();
  shared.release();
  unsubA();
  unsubB();
});

it("keeps a successful cache after a failed refresh and backs off remounts", async () => {
  vi.useFakeTimers();
  const { cachedPreview } = await import("./linkPreviewRequests");
  const data = { title: "Keep this card", description: "", image: null };
  const client = {
    get: vi.fn().mockResolvedValueOnce(data).mockRejectedValue(new ApiError(400, "unavailable")),
  };
  const first = acquirePreview(client, "stale-failed", "https://example.com/stale-failed");
  await first.request;
  first.release();
  await vi.advanceTimersByTimeAsync(300_001);
  const refresh = acquirePreview(client, "stale-failed", "https://example.com/stale-failed");
  expect(await refresh.request).toBe(data);
  refresh.release();
  expect(cachedPreview("stale-failed")).toBe(data);
  const remount = acquirePreview(client, "stale-failed", "https://example.com/stale-failed");
  expect(await remount.request).toBe(data);
  remount.release();
  expect(client.get).toHaveBeenCalledTimes(2);
});

it("keeps stale data when cancelled and immediately resumes refresh for a new view", async () => {
  vi.useFakeTimers();
  const { cachedPreview } = await import("./linkPreviewRequests");
  const data = { title: "Cached before leaving", description: "", image: null };
  const client = {
    get: vi
      .fn()
      .mockResolvedValueOnce(data)
      .mockImplementation(
        (_url, { signal }) =>
          new Promise((_resolve, reject) =>
            signal.addEventListener("abort", () => reject(signal.reason), { once: true }),
          ),
      ),
  };
  const first = acquirePreview(client, "stale-aborted", "https://example.com/stale-aborted");
  await first.request;
  first.release();
  await vi.advanceTimersByTimeAsync(300_001);
  const abandoned = acquirePreview(client, "stale-aborted", "https://example.com/stale-aborted");
  await vi.advanceTimersByTimeAsync(0);
  abandoned.release();
  expect(cachedPreview("stale-aborted")).toBe(data);
  const next = acquirePreview(client, "stale-aborted", "https://example.com/stale-aborted");
  await vi.advanceTimersByTimeAsync(0);
  expect(client.get).toHaveBeenCalledTimes(3);
  next.release();
  await Promise.all([abandoned.request, next.request]);
});
