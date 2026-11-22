import "../../../styles.css";

import {
  RouterProvider,
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  useNavigate,
} from "@tanstack/react-router";
import { cleanup, render, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vite-plus/test";

import { CollectionsProvider } from "../../../lib/electric/collections-context";
import { shouldRestoreScroll } from "../../../router";
import { LocaleProvider } from "../../i18n/LocaleProvider";
import type { FeedMessage } from "./messageFormat";
import { clearChannelMeasurementCache } from "./messageScroll";
import { MessageList } from "./MessageList";
import { ThreadPane } from "./ThreadPane";

const mockCollection = {
  get: () => undefined,
  values: () => [].values(),
  subscribeChanges: () => ({ unsubscribe: () => {} }),
};
const testCollections = {
  config: {},
  emoji: mockCollection,
  users: mockCollection,
  messages: () => mockCollection,
  reactions: () => mockCollection,
  attachments: () => mockCollection,
} as never;

function makeMessage(id: string, index: number, lines = 2, channelId = "chan-a"): FeedMessage {
  const created = new Date(Date.UTC(2026, 0, 1, 0, index))
    .toISOString()
    .replace("T", " ")
    .replace("Z", "");
  const body = Array.from({ length: lines }, (_, line) => `msg ${id} line ${line}`).join("\n");
  return {
    id,
    channel_id: channelId,
    user_id: index % 2 === 0 ? "alice" : "bob",
    parent_id: null,
    message_kind: "message",
    content: body,
    created_at: created,
    updated_at: null,
    deleted_at: null,
    user_name: index % 2 === 0 ? "alice" : "bob",
    display_name: index % 2 === 0 ? "Alice" : "Bob",
  } as unknown as FeedMessage;
}

const noop = () => {};

function listProps(
  channelId: string,
  messages: FeedMessage[],
  overrides: Record<string, unknown> = {},
) {
  return {
    channelId,
    messages,
    isLoading: false,
    isError: false,
    currentUserId: "alice",
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
    onOpenThread: noop,
    hasOlder: false,
    ...overrides,
  };
}

describe("MessageList and ThreadPane initial bottom landing & channel switch", () => {
  let fetchSpy: ReturnType<typeof vi.spyOn>;

  beforeEach(() => {
    clearChannelMeasurementCache();
    fetchSpy = vi.spyOn(globalThis, "fetch").mockImplementation(async (input) => {
      const url =
        typeof input === "string" ? input : input instanceof Request ? input.url : String(input);
      if (url.includes("/read-state")) {
        return new Response(JSON.stringify({ last_read_at: "2027-01-01T00:00:00Z" }), {
          status: 200,
          headers: { "content-type": "application/json" },
        });
      }
      return new Response("{}", { status: 404 });
    });
  });

  afterEach(() => {
    cleanup();
    fetchSpy.mockRestore();
    clearChannelMeasurementCache();
  });

  it("mounts uncached channel near bottom without ever mounting index 0, firing false loadOlder, or showing top-to-bottom scroll", async () => {
    const messages = Array.from({ length: 50 }, (_, index) =>
      makeMessage(`a-${index}`, index, (index % 4) + 2, "chan-uncached"),
    );
    const onLoadOlder = vi.fn();

    const samples: Array<{
      opacity: string;
      scrollTop: number;
      maxTop: number;
      minIndex: number | null;
    }> = [];
    const observer = new MutationObserver(() => {
      const scroller = document.querySelector<HTMLElement>('[data-testid="message-scroll"]');
      if (!scroller) return;
      const indices = Array.from(scroller.querySelectorAll<HTMLElement>("[data-index]")).map((el) =>
        Number(el.dataset.index),
      );
      samples.push({
        opacity: getComputedStyle(scroller).opacity,
        scrollTop: scroller.scrollTop,
        maxTop: Math.max(0, scroller.scrollHeight - scroller.clientHeight),
        minIndex: indices.length > 0 ? Math.min(...indices) : null,
      });
    });
    observer.observe(document.body, { childList: true, subtree: true, attributes: true });

    try {
      render(
        <CollectionsProvider value={testCollections}>
          <LocaleProvider>
            <div style={{ width: 640, height: 420, display: "flex", flexDirection: "column" }}>
              <MessageList
                {...listProps("chan-uncached", messages, {
                  hasOlder: true,
                  onLoadOlder,
                })}
              />
            </div>
          </LocaleProvider>
        </CollectionsProvider>,
      );

      await waitFor(() => {
        const scroller = document.querySelector<HTMLElement>('[data-testid="message-scroll"]');
        expect(scroller).toBeTruthy();
        expect(getComputedStyle(scroller!).opacity).toBe("1");
      });

      const scroller = document.querySelector<HTMLElement>('[data-testid="message-scroll"]')!;
      const maxTop = Math.max(0, scroller.scrollHeight - scroller.clientHeight);
      expect(maxTop).toBeGreaterThan(1000);
      expect(Math.abs(scroller.scrollTop - maxTop)).toBeLessThanOrEqual(2);

      // Should NEVER have triggered loadOlder during initial positioning.
      expect(onLoadOlder).not.toHaveBeenCalled();

      // Index 0 was never mounted on initial render because initialOffset starts near the bottom.
      for (const sample of samples) {
        expect(sample.minIndex).not.toBe(0);
        if (sample.opacity === "1" && sample.maxTop > 0) {
          expect(Math.abs(sample.scrollTop - sample.maxTop)).toBeLessThanOrEqual(2);
        }
      }
    } finally {
      observer.disconnect();
    }
  });

  it("lands cleanly at bottom even when actual row heights are smaller than the 64px estimate", async () => {
    const compact = Array.from({ length: 50 }, (_, index) =>
      makeMessage(`c-${index}`, index, 1, "chan-compact"),
    );

    render(
      <CollectionsProvider value={testCollections}>
        <LocaleProvider>
          <div style={{ width: 640, height: 600, display: "flex", flexDirection: "column" }}>
            <MessageList {...listProps("chan-compact", compact)} />
          </div>
        </LocaleProvider>
      </CollectionsProvider>,
    );

    await waitFor(() => {
      const scroller = document.querySelector<HTMLElement>('[data-testid="message-scroll"]');
      expect(scroller).toBeTruthy();
      expect(getComputedStyle(scroller!).opacity).toBe("1");
    });

    const scroller = document.querySelector<HTMLElement>('[data-testid="message-scroll"]')!;
    const maxTop = Math.max(0, scroller.scrollHeight - scroller.clientHeight);
    expect(maxTop).toBeGreaterThan(500);
    expect(Math.abs(scroller.scrollTop - maxTop)).toBeLessThanOrEqual(2);
    expect(scroller.querySelector('[data-message-id="c-49"]')).toBeTruthy();
  });

  it("reuses cached row heights on channel revisit so return render is synchronously settled at bottom", async () => {
    const messagesA = Array.from({ length: 50 }, (_, index) =>
      makeMessage(`a-${index}`, index, 3, "chan-a"),
    );
    const messagesB = Array.from({ length: 30 }, (_, index) =>
      makeMessage(`b-${index}`, index, 2, "chan-b"),
    );

    const view = render(
      <CollectionsProvider value={testCollections}>
        <LocaleProvider>
          <div style={{ width: 640, height: 420, display: "flex", flexDirection: "column" }}>
            <MessageList key="chan-a" {...listProps("chan-a", messagesA)} />
          </div>
        </LocaleProvider>
      </CollectionsProvider>,
    );

    await waitFor(() => {
      const el = document.querySelector<HTMLElement>('[data-testid="message-scroll"]');
      expect(el).toBeTruthy();
      expect(getComputedStyle(el!).opacity).toBe("1");
    });

    // Switch to channel B (unmounts chan-a cleanly and saves its measurements).
    view.rerender(
      <CollectionsProvider value={testCollections}>
        <LocaleProvider>
          <div style={{ width: 640, height: 420, display: "flex", flexDirection: "column" }}>
            <MessageList key="chan-b" {...listProps("chan-b", messagesB)} />
          </div>
        </LocaleProvider>
      </CollectionsProvider>,
    );

    await waitFor(() => {
      const el = document.querySelector<HTMLElement>('[data-testid="message-scroll"]');
      expect(el?.dataset.scrollRestorationId).toBe("message-scroll:chan-b");
      expect(getComputedStyle(el!).opacity).toBe("1");
    });

    // Switch BACK to channel A cleanly (with cached row heights).
    view.rerender(
      <CollectionsProvider value={testCollections}>
        <LocaleProvider>
          <div style={{ width: 640, height: 420, display: "flex", flexDirection: "column" }}>
            <MessageList key="chan-a" {...listProps("chan-a", messagesA)} />
          </div>
        </LocaleProvider>
      </CollectionsProvider>,
    );

    const returned = document.querySelector<HTMLElement>('[data-testid="message-scroll"]')!;
    expect(returned.dataset.scrollRestorationId).toBe("message-scroll:chan-a");
    expect(getComputedStyle(returned).opacity).toBe("1");
    const maxTop = Math.max(0, returned.scrollHeight - returned.clientHeight);
    expect(maxTop).toBeGreaterThan(1000);
    expect(Math.abs(returned.scrollTop - maxTop)).toBeLessThanOrEqual(2);

    // Switch to B, then back to A with 2 new trailing messages added while away.
    view.rerender(
      <CollectionsProvider value={testCollections}>
        <LocaleProvider>
          <div style={{ width: 640, height: 420, display: "flex", flexDirection: "column" }}>
            <MessageList key="chan-b" {...listProps("chan-b", messagesB)} />
          </div>
        </LocaleProvider>
      </CollectionsProvider>,
    );
    await waitFor(() => {
      const el = document.querySelector<HTMLElement>('[data-testid="message-scroll"]');
      expect(el?.dataset.scrollRestorationId).toBe("message-scroll:chan-b");
    });

    const extendedA = [
      ...messagesA,
      makeMessage("a-50", 50, 4, "chan-a"),
      makeMessage("a-51", 51, 5, "chan-a"),
    ];
    view.rerender(
      <CollectionsProvider value={testCollections}>
        <LocaleProvider>
          <div style={{ width: 640, height: 420, display: "flex", flexDirection: "column" }}>
            <MessageList key="chan-a" {...listProps("chan-a", extendedA)} />
          </div>
        </LocaleProvider>
      </CollectionsProvider>,
    );

    await waitFor(() => {
      const el = document.querySelector<HTMLElement>('[data-testid="message-scroll"]');
      expect(el).toBeTruthy();
      expect(getComputedStyle(el!).opacity).toBe("1");
      const extMax = Math.max(0, el!.scrollHeight - el!.clientHeight);
      expect(extMax).toBeGreaterThan(maxTop);
      expect(Math.abs(el!.scrollTop - extMax)).toBeLessThanOrEqual(2);
      expect(el!.querySelector('[data-message-id="a-51"]')).toBeTruthy();
    });
  });

  it("does not copy scrollTop across channels or restore stale position on router back/forward", async () => {
    const shortMessages = Array.from({ length: 3 }, (_, index) =>
      makeMessage(`s-${index}`, index, 1, "short"),
    );
    const longMessages = Array.from({ length: 50 }, (_, index) =>
      makeMessage(`l-${index}`, index, 3, "long"),
    );

    function ChannelRouteComponent({ channelId }: { channelId: string }) {
      const navigate = useNavigate();
      const messages = channelId === "short" ? shortMessages : longMessages;
      return (
        <div style={{ width: 640, height: 420, display: "flex", flexDirection: "column" }}>
          <button
            type="button"
            onClick={() =>
              void navigate({ to: "/channels/$channelId", params: { channelId: "long" } })
            }
          >
            to-long
          </button>
          <button
            type="button"
            onClick={() =>
              void navigate({ to: "/channels/$channelId", params: { channelId: "short" } })
            }
          >
            to-short
          </button>
          <MessageList key={channelId} {...listProps(channelId, messages)} />
        </div>
      );
    }

    const rootRoute = createRootRoute();
    const channelRoute = createRoute({
      getParentRoute: () => rootRoute,
      path: "/channels/$channelId",
      component: () => {
        const { channelId } = channelRoute.useParams();
        return <ChannelRouteComponent channelId={channelId} />;
      },
    });
    const routeTree = rootRoute.addChildren([channelRoute]);
    const router = createRouter({
      routeTree,
      history: createMemoryHistory({ initialEntries: ["/channels/short"] }),
      scrollRestoration: shouldRestoreScroll,
    });

    render(
      <CollectionsProvider value={testCollections}>
        <LocaleProvider>
          <RouterProvider router={router} />
        </LocaleProvider>
      </CollectionsProvider>,
    );

    await waitFor(() => {
      const el = document.querySelector<HTMLElement>('[data-testid="message-scroll"]');
      expect(el?.dataset.scrollRestorationId).toBe("message-scroll:short");
    });

    // Navigate from short (scrollTop = 0) -> long channel.
    document.querySelector<HTMLButtonElement>("button")?.click();

    await waitFor(() => {
      const el = document.querySelector<HTMLElement>('[data-testid="message-scroll"]');
      expect(el?.dataset.scrollRestorationId).toBe("message-scroll:long");
      expect(getComputedStyle(el!).opacity).toBe("1");
    });

    // Wait past TanStack Router's async onRendered scroll restoration callback.
    await new Promise((resolve) => setTimeout(resolve, 100));

    const longScroller = document.querySelector<HTMLElement>('[data-testid="message-scroll"]')!;
    const longMax = Math.max(0, longScroller.scrollHeight - longScroller.clientHeight);
    expect(longMax).toBeGreaterThan(1000);
    expect(Math.abs(longScroller.scrollTop - longMax)).toBeLessThanOrEqual(2);

    // Navigate back -> forward via router history and confirm it STILL lands at bottom.
    router.history.back();
    await waitFor(() => {
      const el = document.querySelector<HTMLElement>('[data-testid="message-scroll"]');
      expect(el?.dataset.scrollRestorationId).toBe("message-scroll:short");
    });
    router.history.forward();
    await waitFor(() => {
      const el = document.querySelector<HTMLElement>('[data-testid="message-scroll"]');
      expect(el?.dataset.scrollRestorationId).toBe("message-scroll:long");
      expect(getComputedStyle(el!).opacity).toBe("1");
    });
    await new Promise((resolve) => setTimeout(resolve, 80));
    const afterForward = document.querySelector<HTMLElement>('[data-testid="message-scroll"]')!;
    const forwardMax = Math.max(0, afterForward.scrollHeight - afterForward.clientHeight);
    expect(Math.abs(afterForward.scrollTop - forwardMax)).toBeLessThanOrEqual(2);
  });

  it("centers targetMessageId on initial open without flashing bottom or marking channel read", async () => {
    const messages = Array.from({ length: 50 }, (_, index) =>
      makeMessage(`t-${index}`, index, 2, "chan-target"),
    );

    const markReadCalls: string[] = [];
    fetchSpy.mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url =
        typeof input === "string" ? input : input instanceof Request ? input.url : String(input);
      if (url.includes("/read-state")) {
        if ((init?.method ?? "GET").toUpperCase() === "PUT") {
          markReadCalls.push(url);
        }
        return new Response(JSON.stringify({ last_read_at: "2025-01-01T00:00:00Z" }), {
          status: 200,
          headers: { "content-type": "application/json" },
        });
      }
      return new Response("{}", { status: 404 });
    });

    const seenBottomWhileVisible: boolean[] = [];
    const observer = new MutationObserver(() => {
      const scroller = document.querySelector<HTMLElement>('[data-testid="message-scroll"]');
      if (!scroller || getComputedStyle(scroller).opacity !== "1") return;
      const maxTop = Math.max(0, scroller.scrollHeight - scroller.clientHeight);
      if (maxTop > 500 && Math.abs(scroller.scrollTop - maxTop) <= 8) {
        seenBottomWhileVisible.push(true);
      }
    });
    observer.observe(document.body, { childList: true, subtree: true, attributes: true });

    try {
      render(
        <CollectionsProvider value={testCollections}>
          <LocaleProvider>
            <div style={{ width: 640, height: 420, display: "flex", flexDirection: "column" }}>
              <MessageList
                {...listProps("chan-target", messages, {
                  targetMessageId: "t-8",
                })}
              />
            </div>
          </LocaleProvider>
        </CollectionsProvider>,
      );

      await waitFor(() => {
        const scroller = document.querySelector<HTMLElement>('[data-testid="message-scroll"]');
        expect(scroller).toBeTruthy();
        expect(getComputedStyle(scroller!).opacity).toBe("1");
        const target = scroller!.querySelector<HTMLElement>('[data-message-id="t-8"]');
        expect(target).toBeTruthy();
        const sr = scroller!.getBoundingClientRect();
        const tr = target!.getBoundingClientRect();
        expect(tr.top).toBeGreaterThanOrEqual(sr.top - 4);
        expect(tr.bottom).toBeLessThanOrEqual(sr.bottom + 4);
      });

      expect(seenBottomWhileVisible).toEqual([]);
      expect(markReadCalls).toEqual([]);
    } finally {
      observer.disconnect();
    }
  });

  it("opens directly at bottom (or targetMessageId) in ThreadPane without an initial scrollTop=0 flash", async () => {
    const root = makeMessage("root-1", 0, 2, "chan-thread");
    const replies = Array.from(
      { length: 40 },
      (_, index) =>
        ({
          ...makeMessage(`reply-${index}`, index + 1, 4, "chan-thread"),
          parent_id: "root-1",
        }) as FeedMessage,
    );

    const view = render(
      <CollectionsProvider value={testCollections}>
        <LocaleProvider>
          <div style={{ position: "relative", width: 800, height: 480, display: "flex" }}>
            <ThreadPane
              root={root}
              replies={replies}
              isLoading={false}
              isError={false}
              currentUserId="alice"
              reactions={new Map()}
              kitchenCatalog={null}
              canReact={false}
              onReaction={noop}
              onPickReaction={noop}
              editingId={null}
              editingText=""
              onEditingTextChange={noop}
              onStartEdit={noop}
              onCancelEdit={noop}
              onSaveEdit={noop}
              onRequestDelete={noop}
              onSendReply={async () => true}
              onClose={noop}
            />
          </div>
        </LocaleProvider>
      </CollectionsProvider>,
    );

    await waitFor(() => {
      const threadScroller = document.querySelector<HTMLElement>(
        '[data-scroll-restoration-id="thread-scroll:root-1"]',
      );
      expect(threadScroller).toBeTruthy();
      const maxTop = Math.max(0, threadScroller!.scrollHeight - threadScroller!.clientHeight);
      expect(maxTop).toBeGreaterThan(500);
      expect(Math.abs(threadScroller!.scrollTop - maxTop)).toBeLessThanOrEqual(2);
    });

    // With targetMessageId on initial open, centers/aligns on target instead of bottom.
    view.rerender(
      <CollectionsProvider value={testCollections}>
        <LocaleProvider>
          <div style={{ position: "relative", width: 800, height: 480, display: "flex" }}>
            <ThreadPane
              root={{ ...root, id: "root-2" }}
              replies={replies.map((r) => ({ ...r, parent_id: "root-2" }))}
              targetMessageId="reply-3"
              isLoading={false}
              isError={false}
              currentUserId="alice"
              reactions={new Map()}
              kitchenCatalog={null}
              canReact={false}
              onReaction={noop}
              onPickReaction={noop}
              editingId={null}
              editingText=""
              onEditingTextChange={noop}
              onStartEdit={noop}
              onCancelEdit={noop}
              onSaveEdit={noop}
              onRequestDelete={noop}
              onSendReply={async () => true}
              onClose={noop}
            />
          </div>
        </LocaleProvider>
      </CollectionsProvider>,
    );

    await waitFor(() => {
      const threadScroller = document.querySelector<HTMLElement>(
        '[data-scroll-restoration-id="thread-scroll:root-2"]',
      );
      expect(threadScroller).toBeTruthy();
      const targetRow = threadScroller!.querySelector<HTMLElement>('[data-message-id="reply-3"]')!;
      expect(targetRow).toBeTruthy();
      const scrollerRect = threadScroller!.getBoundingClientRect();
      const targetRect = targetRow.getBoundingClientRect();
      expect(targetRect.top).toBeGreaterThanOrEqual(scrollerRect.top - 4);
      expect(targetRect.bottom).toBeLessThanOrEqual(scrollerRect.bottom + 4);
    });
  });
});
