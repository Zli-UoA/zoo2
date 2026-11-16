// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vite-plus/test";
import {
  useActivityPubNotifications,
  type ActivityPubNotification,
} from "./useActivityPubNotifications";
const mocks = vi.hoisted(() => ({
  client: { get: vi.fn(), request: vi.fn() },
  config: {},
  live: undefined as undefined | (() => void),
}));
vi.mock("../lib/electric/collections-context", () => ({
  useCollections: () => ({ config: mocks.config }),
}));
vi.mock("../lib/api/client", () => ({ createConfiguredApiClient: () => mocks.client }));
vi.mock("./useHomeLive", () => ({
  useHomeLive: (callback: () => void) => {
    mocks.live = callback;
  },
}));
const item = (id: string, second: number, read = false): ActivityPubNotification => ({
  id,
  actor_id: "peer",
  display_name: "Peer",
  activity_id: id,
  kind: "like",
  post_id: "post",
  content: "post",
  pending: false,
  is_read: read,
  created_at: `2026-11-01T00:00:${String(second).padStart(2, "0")}Z`,
});
let root: ReturnType<typeof createRoot>;
let result: ReturnType<typeof useActivityPubNotifications>;
function Probe() {
  result = useActivityPubNotifications();
  return null;
}
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.useFakeTimers();
  mocks.client.get.mockReset();
  mocks.client.request.mockReset();
  mocks.client.request.mockResolvedValue({ ok: true });
  root = createRoot(document.createElement("div"));
});
afterEach(async () => {
  await act(async () => root.unmount());
  vi.useRealTimers();
  vi.unstubAllGlobals();
});
it("keeps loaded older pages after a refresh and acknowledges only the selected visible IDs", async () => {
  let read = false;
  mocks.client.get.mockImplementation(async (path: string) =>
    path.includes("before=")
      ? { items: [item("older", 1)], unread_count: read ? 2 : 3, next_cursor: null }
      : {
          items: [item("new", 3, read), item("middle", 2)],
          unread_count: read ? 2 : 3,
          next_cursor: "middle",
        },
  );
  await act(async () => root.render(createElement(Probe)));
  await act(async () => result.loadMore());
  expect(result.items.map((n) => n.id)).toEqual(["new", "middle", "older"]);
  mocks.client.request.mockImplementation(async () => {
    read = true;
    return {};
  });
  await act(async () => result.markRead(["new"]));
  expect(mocks.client.request).toHaveBeenCalledWith("/api/activitypub/notifications/read", {
    method: "POST",
    body: { ids: ["new"] },
  });
  await act(async () => result.refresh());
  expect(result.items.map((n) => n.id)).toEqual(["new", "middle", "older"]);
  expect(result.items.filter((n) => n.is_read).map((n) => n.id)).toEqual(["new"]);
  expect(result.unread_count).toBe(2);
});
it("defers a live refresh during older loading and retains its tail when a newer event arrives", async () => {
  let finish!: (value: unknown) => void;
  let newEvent = false;
  let olderCalls = 0;
  mocks.client.get.mockImplementation(async (path: string) => {
    if (!path.includes("before="))
      return {
        items: newEvent ? [item("arrived", 4), item("a", 3)] : [item("a", 3), item("b", 2)],
        unread_count: 4,
        next_cursor: "cursor",
      };
    if (++olderCalls === 1)
      return new Promise((resolve) => {
        finish = resolve;
      });
    return { items: [item("b", 2), item("c", 1)], unread_count: 4, next_cursor: null };
  });
  await act(async () => root.render(createElement(Probe)));
  let older!: Promise<void>;
  await act(async () => {
    older = result.loadMore();
  });
  newEvent = true;
  await act(async () => {
    mocks.live?.();
    await vi.advanceTimersByTimeAsync(5000);
  });
  expect(mocks.client.get).toHaveBeenCalledTimes(2);
  await act(async () => {
    finish({ items: [item("c", 1)], unread_count: 4, next_cursor: null });
    await older;
  });
  await vi.waitFor(() => expect(result.items.map((n) => n.id)).toEqual(["arrived", "a", "b", "c"]));
  expect(result.next_cursor).toBeNull();
});
it("rolls back a rejected auto-read even if the recovery refresh is offline", async () => {
  mocks.client.get.mockResolvedValue({ items: [item("unread", 1)], unread_count: 4 });
  await act(async () => root.render(createElement(Probe)));
  mocks.client.request.mockRejectedValue(new Error("offline"));
  mocks.client.get.mockRejectedValue(new Error("offline"));
  await act(async () => {
    await expect(result.markRead(["unread"])).rejects.toThrow("offline");
  });
  expect(result.items[0].is_read).toBe(false);
  expect(result.unread_count).toBe(4);
  expect(result.error).toBe(true);
});
it("finishes a slow head request and coalesces live updates without aborting it", async () => {
  let finish!: (value: unknown) => void;
  let signal!: AbortSignal;
  mocks.client.get
    .mockImplementationOnce((_path: string, options: { signal: AbortSignal }) => {
      signal = options.signal;
      return new Promise((resolve) => {
        finish = resolve;
      });
    })
    .mockResolvedValue({ items: [item("fresh", 2)], unread_count: 1 });
  await act(async () => root.render(createElement(Probe)));
  await act(async () => {
    for (let event = 0; event < 10; event++) {
      mocks.live?.();
      await vi.advanceTimersByTimeAsync(1000);
    }
  });
  expect(mocks.client.get).toHaveBeenCalledTimes(1);
  expect(signal.aborted).toBe(false);
  await act(async () => {
    finish({ items: [item("initial", 1)], unread_count: 1 });
  });
  expect(mocks.client.get).toHaveBeenCalledTimes(2);
  expect(result.items.map((row) => row.id)).toEqual(["fresh"]);
  expect(result.loading).toBe(false);
});
it("queues concurrent explicit refreshes and waits for them before loading the latest cursor", async () => {
  mocks.client.get.mockResolvedValueOnce({
    items: [item("a", 3)],
    unread_count: 3,
    next_cursor: "old-cursor",
  });
  await act(async () => root.render(createElement(Probe)));
  let finish!: (value: unknown) => void;
  let signal!: AbortSignal;
  mocks.client.get
    .mockImplementationOnce((_path: string, options: { signal: AbortSignal }) => {
      signal = options.signal;
      return new Promise((resolve) => {
        finish = resolve;
      });
    })
    .mockResolvedValueOnce({ items: [item("a", 3)], unread_count: 3, next_cursor: "new-cursor" })
    .mockResolvedValueOnce({ items: [item("b", 2)], unread_count: 3, next_cursor: null });
  let refresh!: Promise<void>;
  let older!: Promise<void>;
  await act(async () => {
    refresh = result.refresh();
    void result.refresh();
    void result.refresh();
    older = result.loadMore();
  });
  expect(mocks.client.get).toHaveBeenCalledTimes(2);
  expect(signal.aborted).toBe(false);
  await act(async () => {
    finish({ items: [item("a", 3)], unread_count: 3, next_cursor: "old-cursor" });
    await refresh;
    await older;
  });
  expect(mocks.client.get).toHaveBeenCalledTimes(4);
  expect(mocks.client.get.mock.calls[3][0]).toBe(
    "/api/activitypub/notifications?before=new-cursor",
  );
  expect(result.items.map((row) => row.id)).toEqual(["a", "b"]);
});
it("aborts on unmount and cancels scheduled live work", async () => {
  let signal!: AbortSignal;
  mocks.client.get.mockImplementation((_path: string, options: { signal: AbortSignal }) => {
    signal = options.signal;
    return new Promise((_resolve, reject) =>
      signal.addEventListener("abort", () => reject(signal.reason), { once: true }),
    );
  });
  await act(async () => root.render(createElement(Probe)));
  await act(async () => {
    mocks.live?.();
    root.render(null);
  });
  await act(async () => {
    await vi.advanceTimersByTimeAsync(10000);
  });
  expect(signal.aborted).toBe(true);
  expect(mocks.client.get).toHaveBeenCalledTimes(1);
});

it("does not run scheduled live refreshes after the tab becomes hidden", async () => {
  mocks.client.get.mockResolvedValue({ items: [], unread_count: 0 });
  await act(async () => root.render(createElement(Probe)));
  await act(async () => {
    mocks.live?.();
  });
  const visibility = vi.spyOn(document, "visibilityState", "get").mockReturnValue("hidden");
  await act(async () => {
    await vi.advanceTimersByTimeAsync(5000);
  });
  expect(mocks.client.get).toHaveBeenCalledTimes(1);
  visibility.mockRestore();
  await act(async () => {
    mocks.live?.();
    await vi.advanceTimersByTimeAsync(5000);
  });
  expect(mocks.client.get).toHaveBeenCalledTimes(2);
});

it("refreshes only the head and visible older notifications, preserving the pagination cursor", async () => {
  const head = Array.from({ length: 100 }, (_, index) => item(`head-${index}`, 3));
  const history = [item("older", 1), item("deleted", 0)];
  mocks.client.get.mockImplementation(async (path: string) =>
    path.includes("before=")
      ? { items: history, unread_count: 102, next_cursor: "tail" }
      : { items: head, unread_count: 102, next_cursor: "head-99" },
  );
  mocks.client.request.mockImplementation(async (path: string) => {
    expect(path).toBe("/api/activitypub/notifications/entries");
    return { items: [{ ...history[0], content: "updated" }], unread_count: 102 };
  });
  await act(async () => root.render(createElement(Probe)));
  await act(async () => result.loadMore());
  act(() => {
    result.setVisible("older", true);
    result.setVisible("deleted", true);
  });
  mocks.client.get.mockClear();
  await act(async () => result.refresh());
  expect(mocks.client.get).toHaveBeenCalledTimes(1);
  expect(mocks.client.request).toHaveBeenCalledWith(
    "/api/activitypub/notifications/entries",
    expect.objectContaining({ method: "POST", body: { entries: ["older", "deleted"] } }),
  );
  expect(result.items.find((n) => n.id === "older")?.content).toBe("updated");
  expect(result.items.some((n) => n.id === "deleted")).toBe(false);
  expect(result.next_cursor).toBe("tail");
});

it.each([null, "old-tail"])(
  "fills a disjoint refreshed head before resuming the old cursor %s",
  async (tail) => {
    const rows = (from: number, to: number) =>
      Array.from({ length: from - to + 1 }, (_, i) => ({
        ...item(`id-${String(from - i).padStart(6, "0")}`, 0),
        created_at: new Date(Date.UTC(2026, 10, 1, 0, 0, from - i)).toISOString(),
      }));
    let fresh = false;
    mocks.client.get.mockImplementation(async (path: string) => {
      if (path.includes("before=old-head"))
        return { items: rows(200, 101), next_cursor: tail, unread_count: 200 };
      if (path.includes("before=new-head"))
        return { items: rows(350, 251), next_cursor: "bridge", unread_count: 350 };
      return {
        items: fresh ? rows(450, 351) : rows(300, 201),
        next_cursor: fresh ? "new-head" : "old-head",
        unread_count: 350,
      };
    });
    await act(async () => root.render(createElement(Probe)));
    await act(async () => result.loadMore());
    fresh = true;
    await act(async () => result.refresh());
    expect(result.next_cursor).toBe("new-head");
    await act(async () => result.loadMore());
    expect(result.items.map((n) => n.id)).toEqual(rows(450, 101).map((n) => n.id));
    expect(result.next_cursor).toBe(tail);
  },
);

it.each([false, true])(
  "keeps a completed read and counter during a stale response (append=%s)",
  async (append) => {
    mocks.client.get
      .mockResolvedValueOnce({ items: [item("head", 3)], next_cursor: "head", unread_count: 2 })
      .mockResolvedValueOnce({ items: [item("older", 1)], next_cursor: "older", unread_count: 2 });
    await act(async () => root.render(createElement(Probe)));
    await act(async () => result.loadMore());
    let finish!: (value: unknown) => void;
    mocks.client.get.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    );
    let task!: Promise<void>;
    act(() => {
      task = append ? result.loadMore() : result.refresh();
    });
    await act(async () => result.markRead(["older"]));
    await act(async () => {
      finish({ items: [item("head", 3)], next_cursor: "head", unread_count: 2 });
      await task;
    });
    expect(result.items.find((n) => n.id === "older")?.is_read).toBe(true);
    expect(result.unread_count).toBe(1);
  },
);
