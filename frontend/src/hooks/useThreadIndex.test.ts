// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vite-plus/test";
import { useThreadIndex, type ThreadSummary, type ThreadPage } from "./useThreadIndex";
import { THREADS_CHANGED } from "../lib/electric/threads";
import { createMessageWrites } from "../lib/electric/writes";
import type { ApiClient } from "../lib/api/client";

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
function page(items: ThreadSummary[], next_cursor: string | null = null): ThreadPage {
  return { items, next_cursor };
}
function Test({
  userId,
  ...options
}: {
  userId: string | null;
  archived?: boolean;
  enabled?: boolean;
  channelId?: string;
}) {
  index = useThreadIndex(userId, options);
  return null;
}
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.useFakeTimers();
  fixture.api.get.mockReset();
  fixture.api.post.mockReset().mockResolvedValue({ txid: "42" });
  root = createRoot(document.createElement("div"));
});
afterEach(async () => {
  await act(async () => root.unmount());
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

it("waits for the user and clears the previous user's results while the next request is pending", async () => {
  fixture.api.get.mockResolvedValue(page([thread]));
  await act(async () => root.render(createElement(Test, { userId: null })));
  expect(fixture.api.get).not.toHaveBeenCalled();
  await act(async () => root.render(createElement(Test, { userId: "alice" })));
  expect(index.threads).toEqual([thread]);
  let resolve!: (threads: ThreadPage) => void;
  fixture.api.get.mockImplementationOnce(
    () =>
      new Promise((done) => {
        resolve = done;
      }),
  );
  await act(async () => root.render(createElement(Test, { userId: "bob" })));
  expect(index.threads).toEqual([]);
  await act(async () => resolve(page([])));
  expect(index.threads).toEqual([]);
});

it("serializes refreshes and discards a response started before a reply was saved", async () => {
  let resolve!: (threads: ThreadPage) => void;
  fixture.api.get
    .mockImplementationOnce(
      () =>
        new Promise((done) => {
          resolve = done;
        }),
    )
    .mockResolvedValue(page([{ ...thread, thread_archive_at: null }]));
  await act(async () => root.render(createElement(Test, { userId: "alice" })));
  await act(async () => {
    await vi.advanceTimersByTimeAsync(60_000);
  });
  expect(fixture.api.get).toHaveBeenCalledTimes(1);
  await act(async () => window.dispatchEvent(new Event(THREADS_CHANGED)));
  await act(async () => resolve(page([thread])));
  expect(fixture.api.get).toHaveBeenCalledTimes(2);
  expect(index.threads[0].thread_archive_at).toBeNull();
});

it("keeps the previous list on failure and refreshes after online recovery or a saved reply", async () => {
  fixture.api.get
    .mockResolvedValueOnce(page([thread]))
    .mockRejectedValueOnce(new Error("offline"))
    .mockResolvedValue(page([{ ...thread, thread_archive_at: null }]));
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

it("shows a new thread after its first reply is saved without waiting for polling or Electric", async () => {
  const activeThread = { ...thread, thread_archive_at: null, reply_count: 1 };
  fixture.api.get.mockResolvedValueOnce(page([])).mockResolvedValue(page([activeThread]));
  await act(async () => root.render(createElement(Test, { userId: "alice" })));
  expect(index.threads).toEqual([]);
  let confirmSync!: (result: boolean) => void;
  const awaitTxId = vi.fn(
    () =>
      new Promise<boolean>((resolve) => {
        confirmSync = resolve;
      }),
  );
  const writes = createMessageWrites({
    client: fixture.api as unknown as ApiClient,
    txidMode: "awaitTxId",
    getCurrentUserId: () => "alice",
  });
  let saving!: Promise<void>;
  await act(async () => {
    saving = writes.onInsert({
      transaction: {
        mutations: [
          {
            modified: {
              id: "reply",
              channel_id: thread.channel_id,
              parent_id: thread.id,
              content: "First reply",
            },
          },
        ],
      },
      collection: { utils: { awaitTxId } },
    } as never);
  });
  expect(index.threads).toEqual([activeThread]);
  expect(fixture.api.get).toHaveBeenCalledTimes(2);
  expect(awaitTxId).toHaveBeenCalled();
  await act(async () => {
    confirmSync(true);
    await saving;
  });
});

it("fetches archive pages only while expanded and does not poll history", async () => {
  fixture.api.get
    .mockResolvedValueOnce(page([thread], "older / cursor"))
    .mockResolvedValue(page([{ ...thread, id: "older" }]));
  const view = (enabled: boolean) =>
    createElement(Test, { userId: "alice", channelId: "general", archived: true, enabled });
  await act(async () => root.render(view(false)));
  expect(fixture.api.get).not.toHaveBeenCalled();
  await act(async () => root.render(view(true)));
  expect(fixture.api.get).toHaveBeenCalledWith("/api/threads?channel_id=general&archived=true");
  await act(async () => vi.advanceTimersByTimeAsync(120_000));
  expect(fixture.api.get).toHaveBeenCalledTimes(1);
  await act(async () => index.loadMore());
  expect(fixture.api.get).toHaveBeenLastCalledWith(
    "/api/threads?channel_id=general&archived=true&before=older+%2F+cursor",
  );
  expect(index.threads.map((entry) => entry.id)).toEqual([thread.id, "older"]);
  expect(index.hasMore).toBe(false);
});

it("keeps a failed archive page retryable without losing loaded history", async () => {
  fixture.api.get
    .mockResolvedValueOnce(page([thread], "older"))
    .mockRejectedValueOnce(new Error("Offline"))
    .mockResolvedValue(page([{ ...thread, id: "older" }]));
  await act(async () => root.render(createElement(Test, { userId: "alice", archived: true })));
  await act(async () => index.loadMore());
  expect(index.isError).toBe(true);
  expect(index.threads).toEqual([thread]);
  await act(async () => index.loadMore());
  expect(index.isError).toBe(false);
  expect(index.threads).toHaveLength(2);
});
