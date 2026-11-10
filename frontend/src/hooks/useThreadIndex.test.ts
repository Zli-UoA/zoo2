// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vite-plus/test";
import { THREADS_CHANGED, useThreadIndex, type ThreadSummary } from "./useThreadIndex";

const fixture = vi.hoisted(() => ({ api: { get: vi.fn(), post: vi.fn() }, config: {} }));
vi.mock("../lib/api/client", () => ({ createConfiguredApiClient: () => fixture.api }));
vi.mock("../lib/electric/collections-context", () => ({
  useCollections: () => ({ config: fixture.config }),
}));

const thread: ThreadSummary = {
  id: "thread",
  channel_id: "general",
  content: "Thread",
  thread_archive_at: "2026-01-08T00:00:00Z",
  reply_count: 2,
};
let index: ReturnType<typeof useThreadIndex>;
let root: ReturnType<typeof createRoot>;
function Test({ userId }: { userId: string | null }) {
  index = useThreadIndex(userId);
  return null;
}
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.useFakeTimers();
  fixture.api.get.mockReset();
  fixture.api.post.mockReset().mockResolvedValue({});
  root = createRoot(document.createElement("div"));
});
afterEach(async () => {
  await act(async () => root.unmount());
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

it("waits for the user and clears the previous user's results while the next request is pending", async () => {
  fixture.api.get.mockResolvedValue([thread]);
  await act(async () => root.render(createElement(Test, { userId: null })));
  expect(fixture.api.get).not.toHaveBeenCalled();
  await act(async () => root.render(createElement(Test, { userId: "alice" })));
  expect(index.threads).toEqual([thread]);
  let resolve!: (threads: ThreadSummary[]) => void;
  fixture.api.get.mockImplementationOnce(
    () =>
      new Promise((done) => {
        resolve = done;
      }),
  );
  await act(async () => root.render(createElement(Test, { userId: "bob" })));
  expect(index.threads).toEqual([]);
  await act(async () => resolve([]));
  expect(index.threads).toEqual([]);
});

it("serializes refreshes and discards a response started before reopening", async () => {
  let resolve!: (threads: ThreadSummary[]) => void;
  fixture.api.get
    .mockImplementationOnce(
      () =>
        new Promise((done) => {
          resolve = done;
        }),
    )
    .mockResolvedValue([{ ...thread, thread_archive_at: null }]);
  await act(async () => root.render(createElement(Test, { userId: "alice" })));
  await act(async () => {
    await vi.advanceTimersByTimeAsync(60_000);
  });
  expect(fixture.api.get).toHaveBeenCalledTimes(1);
  await act(async () => index.unarchive(thread.id));
  await act(async () => resolve([thread]));
  expect(fixture.api.get).toHaveBeenCalledTimes(2);
  expect(index.threads[0].thread_archive_at).toBeNull();
});

it("keeps the previous list on failure and refreshes after online recovery or a pane reopening", async () => {
  fixture.api.get
    .mockResolvedValueOnce([thread])
    .mockRejectedValueOnce(new Error("offline"))
    .mockResolvedValue([{ ...thread, thread_archive_at: null }]);
  await act(async () => root.render(createElement(Test, { userId: "alice" })));
  await act(async () => {
    window.dispatchEvent(new Event("focus"));
  });
  expect(index.threads).toEqual([thread]);
  await act(async () => {
    window.dispatchEvent(new Event("online"));
  });
  expect(index.threads[0].thread_archive_at).toBeNull();
  await act(async () => {
    window.dispatchEvent(new Event(THREADS_CHANGED));
  });
  expect(fixture.api.get).toHaveBeenCalledTimes(4);
});
