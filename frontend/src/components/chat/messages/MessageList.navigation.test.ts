// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vite-plus/test";
import { LocaleProvider } from "../../i18n/LocaleProvider";
import { MessageList } from "./MessageList";
import type { FeedMessage } from "./messageFormat";

const mocks = vi.hoisted(() => ({ scroll: vi.fn(), markRead: vi.fn() }));
vi.mock("@tanstack/react-virtual", () => {
  const virtualizer = {
    scrollToIndex: mocks.scroll,
    getVirtualItems: () => [],
    getTotalSize: () => 1000,
    measure() {},
  };
  return { useVirtualizer: () => virtualizer };
});
vi.mock("./useMessageReadState", () => ({
  useMessageReadState: () => ({
    firstUnreadId: null,
    markRead: mocks.markRead,
    isLoading: false,
    isError: false,
  }),
}));

it("jumps to a notification target once and preserves position when more messages arrive", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.useFakeTimers();
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const message = (id: string): FeedMessage => ({
    id,
    user_id: "other",
    content: id,
    created_at: "2026-10-24T00:00:00Z",
    parent_id: null,
  });
  const noop = () => {};
  const render = (messages: FeedMessage[], targetMessageId = "target") =>
    root.render(
      createElement(LocaleProvider, {
        children: createElement(MessageList, {
          messages,
          targetMessageId,
          isLoading: false,
          isError: false,
          currentUserId: "me",
          editingId: null,
          editingText: "",
          onEditingTextChange: noop,
          onStartEdit: noop,
          onCancelEdit: noop,
          onSaveEdit: noop,
          onRequestDelete: noop,
          reactions: new Map(),
          kitchenCatalog: null,
          canReact: false,
          onReaction: noop,
          onPickReaction: noop,
          replyCounts: new Map(),
          onOpenThread: noop,
        }),
      }),
    );
  const first = [message("older"), message("target"), message("latest")];
  try {
    await act(async () => render(first));
    await act(async () => {
      vi.advanceTimersByTime(20);
    });
    expect(mocks.scroll).toHaveBeenLastCalledWith(1, { align: "center" });
    mocks.scroll.mockClear();
    await act(async () => render([...first, message("new")]));
    await act(async () => {
      vi.advanceTimersByTime(20);
    });
    expect(mocks.scroll).not.toHaveBeenCalled();
    await act(async () => render(first, "older"));
    await act(async () => {
      vi.advanceTimersByTime(20);
    });
    expect(mocks.scroll).toHaveBeenLastCalledWith(0, { align: "center" });
  } finally {
    await act(async () => root.unmount());
    container.remove();
    vi.useRealTimers();
    vi.unstubAllGlobals();
  }
});
