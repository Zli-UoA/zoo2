// @vitest-environment happy-dom
import { act, createElement, useEffect } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, expect, it, vi } from "vite-plus/test";
import type { Transaction } from "@tanstack/react-db";
import { useWriteRunner } from "../../../hooks/useWriteRunner";
import type { FeedMessage } from "./messageFormat";
import { buildMessageItems } from "./messageGrouping";
import { firstUnreadMessage, useMessageReadState } from "./useMessageReadState";

const api = vi.hoisted(() => ({ get: vi.fn(), post: vi.fn() }));
vi.mock("../../../lib/api/client", () => ({ createConfiguredApiClient: () => api }));
vi.mock("../../../lib/electric/collections", () => ({ getCollections: () => ({ config: {} }) }));
vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));
vi.mock("sonner", () => ({ toast: { error: vi.fn() } }));
beforeEach(() => {
  api.get.mockReset();
  api.post.mockReset();
});
const message = (id: string, user_id = "other"): FeedMessage => ({
  id,
  user_id,
  content: id,
  created_at: `2026-10-23 00:00:0${id}`,
  parent_id: null,
});
const cursor = (n: string) => ({
  channel_id: "channel",
  last_read_at: `2026-10-23T00:00:0${n}`,
  has_unread: n === "1",
});

it("finds unread incoming messages and splits consecutive authors at the boundary", () => {
  expect(
    firstUnreadMessage(
      [message("1"), message("2", "me"), message("3")],
      "me",
      Date.parse("2026-10-23T00:00:01Z"),
    ),
  ).toBe("3");
  const items = buildMessageItems([message("3"), message("4")], undefined, "4");
  expect(items[0].isGroupEnd).toBe(true);
  expect(items[1].isGroupStart).toBe(true);
});

it("saves to the server, retains the visit boundary, and ignores hidden tabs", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.spyOn(document, "hasFocus").mockReturnValue(true);
  const visibility = vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
  const root = createRoot(document.createElement("div"));
  api.get.mockResolvedValue(cursor("1"));
  api.post.mockResolvedValue({ data: cursor("3"), txid: "1" });
  let state!: ReturnType<typeof useMessageReadState>;
  function Test({ pending = false }: { pending?: boolean }) {
    state = useMessageReadState("channel", "me", [message("1"), message("3")], pending);
    return null;
  }
  try {
    await act(async () => root.render(createElement(Test, { pending: true })));
    state.markRead();
    expect(api.post).not.toHaveBeenCalled();
    expect(api.get).toHaveBeenCalledWith("/api/channels/channel/read-state");
    await act(async () => root.render(createElement(Test)));
    expect(state.firstUnreadId).toBe("3");
    visibility.mockReturnValue("hidden");
    state.markRead();
    expect(api.post).not.toHaveBeenCalled();
    visibility.mockReturnValue("visible");
    await act(async () => state.markRead());
    expect(api.post).toHaveBeenCalledWith("/api/channels/channel/read-state", { message_id: "3" });
    expect(state.firstUnreadId).toBe("3");
    state.markRead();
    expect(api.post).toHaveBeenCalledTimes(1);
    api.get.mockResolvedValue(cursor("3"));
    await act(async () => root.render(createElement(Test, { key: "another-browser" })));
    expect(state.firstUnreadId).toBeNull();
  } finally {
    await act(async () => root.unmount());
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  }
});

it("does not mark messages read when fetching the server cursor fails", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  api.get.mockRejectedValue(new Error("offline"));
  const root = createRoot(document.createElement("div"));
  let state!: ReturnType<typeof useMessageReadState>;
  function Test() {
    state = useMessageReadState("channel", "me", [message("1")], false);
    return null;
  }
  try {
    await act(async () => root.render(createElement(Test)));
    expect(state.isError).toBe(true);
    state.markRead();
    expect(api.post).not.toHaveBeenCalled();
  } finally {
    await act(async () => root.unmount());
    vi.unstubAllGlobals();
  }
});

it.each([false, true])("waits for all writes and resumes after rollback=%s", async (rollback) => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.spyOn(document, "hasFocus").mockReturnValue(true);
  vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
  api.get.mockResolvedValue(cursor("1"));
  api.post.mockResolvedValue({ data: cursor("3"), txid: "1" });
  const root = createRoot(document.createElement("div"));
  let runner!: ReturnType<typeof useWriteRunner>;
  let state!: ReturnType<typeof useMessageReadState>;
  const deferred = () => {
    let resolve!: () => void;
    let reject!: (error: Error) => void;
    const promise = new Promise<void>((yes, no) => {
      resolve = yes;
      reject = no;
    });
    return { resolve, reject, transaction: { when: () => promise } as unknown as Transaction };
  };
  const first = deferred();
  const second = deferred();
  let firstRun!: Promise<void>;
  let secondRun!: Promise<void>;
  function Test({ optimistic = false }: { optimistic?: boolean }) {
    runner = useWriteRunner();
    state = useMessageReadState(
      "channel",
      "me",
      [message("1"), ...(optimistic ? [message("3", "me")] : [])],
      false,
      runner.isPending,
    );
    // MessageList reevaluates markRead when pending writes finish.
    useEffect(() => state.markRead(), [state.markRead]);
    return null;
  }
  try {
    await act(async () => root.render(createElement(Test)));
    await act(async () => {
      firstRun = runner.run(() => first.transaction);
      secondRun = runner.run(() => second.transaction);
      root.render(createElement(Test, { optimistic: true }));
    });
    expect(runner.isPending).toBe(true);
    expect(state.firstUnreadId).toBeNull();
    expect(api.post).not.toHaveBeenCalled();
    await act(async () => {
      first.resolve();
      await firstRun;
    });
    expect(runner.isPending).toBe(true);
    state.markRead();
    expect(api.post).not.toHaveBeenCalled();
    await act(async () => {
      if (rollback) {
        root.render(createElement(Test));
        second.reject(new Error("save failed"));
      } else second.resolve();
      await secondRun;
    });
    expect(runner.isPending).toBe(false);
    if (rollback) {
      expect(api.post).not.toHaveBeenCalled();
      expect(runner.error).toBe("save failed");
    } else {
      expect(api.post).toHaveBeenCalledTimes(1);
      expect(api.post).toHaveBeenCalledWith("/api/channels/channel/read-state", {
        message_id: "3",
      });
    }
  } finally {
    await act(async () => root.unmount());
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  }
});
