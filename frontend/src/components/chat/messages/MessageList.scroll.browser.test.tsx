import "../../../styles.css";

import {
  RouterProvider,
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  useNavigate,
} from "@tanstack/react-router";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vite-plus/test";

import { CollectionsProvider } from "../../../lib/electric/collections-context";
import { shouldRestoreScroll } from "../../../router";
import { LocaleProvider } from "../../i18n/LocaleProvider";
import type { FeedMessage } from "./messageFormat";
import {
  buildChannelScrollKey,
  clearChannelMeasurementCache,
  getChannelMeasurementCache,
} from "./messageScroll";
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

    const minIndices: Array<number | null> = [];
    const observer = new MutationObserver(() => {
      const scroller = document.querySelector<HTMLElement>('[data-testid="message-scroll"]');
      if (!scroller) return;
      const indices = Array.from(scroller.querySelectorAll<HTMLElement>("[data-index]")).map((el) =>
        Number(el.dataset.index),
      );
      minIndices.push(indices.length > 0 ? Math.min(...indices) : null);
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

      // Remain synchronously visible (opacity === "1") from initial mount so toBeVisible() checks never race.
      const immediateScroller = document.querySelector<HTMLElement>(
        '[data-testid="message-scroll"]',
      )!;
      expect(immediateScroller).toBeTruthy();
      expect(getComputedStyle(immediateScroller).opacity).toBe("1");

      const rafSamples: Array<{ scrollTop: number; maxTop: number; minIndex: number | null }> = [];
      let framesRemaining = 6;
      await new Promise<void>((resolve) => {
        const capture = () => {
          const indices = Array.from(
            immediateScroller.querySelectorAll<HTMLElement>("[data-index]"),
          ).map((el) => Number(el.dataset.index));
          rafSamples.push({
            scrollTop: immediateScroller.scrollTop,
            maxTop: Math.max(0, immediateScroller.scrollHeight - immediateScroller.clientHeight),
            minIndex: indices.length > 0 ? Math.min(...indices) : null,
          });
          framesRemaining -= 1;
          if (framesRemaining <= 0) resolve();
          else requestAnimationFrame(capture);
        };
        requestAnimationFrame(capture);
      });

      await waitFor(() => {
        const maxTop = Math.max(0, immediateScroller.scrollHeight - immediateScroller.clientHeight);
        expect(maxTop).toBeGreaterThan(1000);
        expect(Math.abs(immediateScroller.scrollTop - maxTop)).toBeLessThanOrEqual(2);
      });

      // Should NEVER have triggered loadOlder during initial positioning.
      expect(onLoadOlder).not.toHaveBeenCalled();

      // Index 0 was never mounted on initial render or any frame because initialOffset starts near bottom.
      for (const minIndex of minIndices) {
        expect(minIndex).not.toBe(0);
      }
      for (const sample of rafSamples) {
        expect(sample.minIndex).not.toBe(0);
        expect(sample.scrollTop).toBeGreaterThan(0);
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
      const maxTop = Math.max(0, scroller!.scrollHeight - scroller!.clientHeight);
      expect(maxTop).toBeGreaterThan(500);
      expect(Math.abs(scroller!.scrollTop - maxTop)).toBeLessThanOrEqual(2);
      expect(scroller!.querySelector('[data-message-id="c-49"]')).toBeTruthy();
    });
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
      const maxTop = Math.max(0, el!.scrollHeight - el!.clientHeight);
      expect(maxTop).toBeGreaterThan(1000);
      expect(Math.abs(el!.scrollTop - maxTop)).toBeLessThanOrEqual(2);
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
      const maxTop = Math.max(0, el!.scrollHeight - el!.clientHeight);
      expect(maxTop).toBeGreaterThan(500);
      expect(Math.abs(el!.scrollTop - maxTop)).toBeLessThanOrEqual(2);
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
      const longMax = Math.max(0, el!.scrollHeight - el!.clientHeight);
      expect(longMax).toBeGreaterThan(1000);
      expect(Math.abs(el!.scrollTop - longMax)).toBeLessThanOrEqual(2);
    });

    // Wait past TanStack Router's async onRendered scroll restoration callback.
    await new Promise((resolve) => setTimeout(resolve, 100));

    const longScroller = document.querySelector<HTMLElement>('[data-testid="message-scroll"]')!;
    const longMax = Math.max(0, longScroller.scrollHeight - longScroller.clientHeight);
    expect(longMax).toBeGreaterThan(1000);
    expect(Math.abs(longScroller.scrollTop - longMax)).toBeLessThanOrEqual(2);

    // Scroll to a distinct mid-list offset before navigating away so router restoration would move to 300 if enabled.
    longScroller.scrollTop = 300;
    fireEvent.scroll(longScroller);
    await new Promise((resolve) => setTimeout(resolve, 60));

    // Navigate back -> forward via router history and confirm it lands at bottom rather than restoring 300.
    router.history.back();
    await waitFor(() => {
      const el = document.querySelector<HTMLElement>('[data-testid="message-scroll"]');
      expect(el?.dataset.scrollRestorationId).toBe("message-scroll:short");
    });
    router.history.forward();
    await waitFor(() => {
      const el = document.querySelector<HTMLElement>('[data-testid="message-scroll"]');
      expect(el?.dataset.scrollRestorationId).toBe("message-scroll:long");
      const forwardMax = Math.max(0, el!.scrollHeight - el!.clientHeight);
      expect(forwardMax).toBeGreaterThan(1000);
      expect(Math.abs(el!.scrollTop - forwardMax)).toBeLessThanOrEqual(2);
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
    const secondLastIso = new Date(Date.UTC(2026, 0, 1, 0, 48)).toISOString();
    const hasFocusSpy = vi.spyOn(document, "hasFocus").mockReturnValue(true);
    fetchSpy.mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url =
        typeof input === "string" ? input : input instanceof Request ? input.url : String(input);
      const method = (
        init?.method ?? (input instanceof Request ? input.method : "GET")
      ).toUpperCase();
      if (url.includes("/read-state")) {
        if (method === "POST") {
          markReadCalls.push(url);
        }
        return new Response(JSON.stringify({ last_read_at: secondLastIso }), {
          status: 200,
          headers: { "content-type": "application/json" },
        });
      }
      return new Response("{}", { status: 404 });
    });

    const seenBottom: boolean[] = [];
    const observer = new MutationObserver(() => {
      const scroller = document.querySelector<HTMLElement>('[data-testid="message-scroll"]');
      if (!scroller) return;
      const maxTop = Math.max(0, scroller.scrollHeight - scroller.clientHeight);
      if (maxTop > 500 && Math.abs(scroller.scrollTop - maxTop) <= 8) {
        seenBottom.push(true);
      }
    });
    observer.observe(document.body, { childList: true, subtree: true, attributes: true });

    try {
      const view = render(
        <CollectionsProvider value={testCollections}>
          <LocaleProvider>
            <div style={{ width: 640, height: 420, display: "flex", flexDirection: "column" }}>
              <MessageList
                key="chan-target"
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
        const target = scroller!.querySelector<HTMLElement>('[data-message-id="t-8"]');
        expect(target).toBeTruthy();
        const sr = scroller!.getBoundingClientRect();
        const tr = target!.getBoundingClientRect();
        expect(tr.top).toBeGreaterThanOrEqual(sr.top - 4);
        expect(tr.bottom).toBeLessThanOrEqual(sr.bottom + 4);
      });
      await new Promise((resolve) => setTimeout(resolve, 60));

      expect(seenBottom).toEqual([]);
      expect(markReadCalls).toEqual([]);

      // Control case: same read-state POST mock DOES record markRead when opening at bottom without targetMessageId.
      view.rerender(
        <CollectionsProvider value={testCollections}>
          <LocaleProvider>
            <div style={{ width: 640, height: 420, display: "flex", flexDirection: "column" }}>
              <MessageList
                key="chan-target-control"
                {...listProps("chan-target-control", messages)}
              />
            </div>
          </LocaleProvider>
        </CollectionsProvider>,
      );

      await waitFor(() => {
        expect(markReadCalls.length).toBeGreaterThanOrEqual(1);
      });
    } finally {
      observer.disconnect();
      hasFocusSpy.mockRestore();
    }
  });

  it("lands at bottom even if targetMessageId is absent from items (e.g., reply or deleted message)", async () => {
    const messages = Array.from({ length: 50 }, (_, index) =>
      makeMessage(`m-${index}`, index, 2, "chan-missing-target"),
    );

    render(
      <CollectionsProvider value={testCollections}>
        <LocaleProvider>
          <div style={{ width: 640, height: 420, display: "flex", flexDirection: "column" }}>
            <MessageList
              {...listProps("chan-missing-target", messages, {
                targetMessageId: "reply-not-in-main-feed",
              })}
            />
          </div>
        </LocaleProvider>
      </CollectionsProvider>,
    );

    await waitFor(() => {
      const scroller = document.querySelector<HTMLElement>('[data-testid="message-scroll"]');
      expect(scroller).toBeTruthy();
      const maxTop = Math.max(0, scroller!.scrollHeight - scroller!.clientHeight);
      expect(maxTop).toBeGreaterThan(500);
      expect(Math.abs(scroller!.scrollTop - maxTop)).toBeLessThanOrEqual(2);
      expect(scroller!.querySelector('[data-message-id="m-49"]')).toBeTruthy();
    });
  });

  it("keeps the last tall unread message visible and marks read when opening at bottom", async () => {
    // Index 19 is created by bob ("other"), 6+ lines (>120px) so initial 64px estimate underestimates height.
    const tallMessages = Array.from({ length: 20 }, (_, index) =>
      makeMessage(`tall-${index}`, index, 7, "chan-last-unread"),
    );
    const secondLastIso = new Date(Date.UTC(2026, 0, 1, 0, 18)).toISOString();
    const markReadCalls: string[] = [];
    const hasFocusSpy = vi.spyOn(document, "hasFocus").mockReturnValue(true);
    fetchSpy.mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url =
        typeof input === "string" ? input : input instanceof Request ? input.url : String(input);
      const method = (
        init?.method ?? (input instanceof Request ? input.method : "GET")
      ).toUpperCase();
      if (url.includes("/read-state")) {
        if (method === "POST") {
          markReadCalls.push(url);
        }
        return new Response(JSON.stringify({ last_read_at: secondLastIso }), {
          status: 200,
          headers: { "content-type": "application/json" },
        });
      }
      return new Response("{}", { status: 404 });
    });

    try {
      render(
        <CollectionsProvider value={testCollections}>
          <LocaleProvider>
            <div style={{ width: 640, height: 420, display: "flex", flexDirection: "column" }}>
              <MessageList {...listProps("chan-last-unread", tallMessages)} />
            </div>
          </LocaleProvider>
        </CollectionsProvider>,
      );

      await waitFor(() => {
        const scroller = document.querySelector<HTMLElement>('[data-testid="message-scroll"]');
        expect(scroller).toBeTruthy();
        const lastRow = scroller!.querySelector<HTMLElement>('[data-message-id="tall-19"]');
        expect(lastRow).toBeTruthy();
        const sr = scroller!.getBoundingClientRect();
        const lr = lastRow!.getBoundingClientRect();
        expect(lr.height).toBeGreaterThan(64);
        expect(lr.top).toBeGreaterThanOrEqual(sr.top - 4);
        expect(lr.bottom).toBeLessThanOrEqual(sr.bottom + 4);
        expect(markReadCalls.length).toBeGreaterThanOrEqual(1);
      });
    } finally {
      hasFocusSpy.mockRestore();
    }
  });

  it("jumps once to the unread boundary and blocks markRead when read-state resolves after initial render", async () => {
    const messages = Array.from({ length: 50 }, (_, index) =>
      makeMessage(`d-${index}`, index, 2, "chan-delayed-unread"),
    );
    // Index 10 is index % 2 === 0 ("alice"), so use index 11 ("bob") as the first unread message from another user.
    const justBefore11 = new Date(Date.UTC(2026, 0, 1, 0, 10, 30)).toISOString();
    let resolveReadState!: () => void;
    const gate = new Promise<void>((resolve) => {
      resolveReadState = resolve;
    });
    const markReadCalls: string[] = [];
    const hasFocusSpy = vi.spyOn(document, "hasFocus").mockReturnValue(true);

    fetchSpy.mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url =
        typeof input === "string" ? input : input instanceof Request ? input.url : String(input);
      const method = (
        init?.method ?? (input instanceof Request ? input.method : "GET")
      ).toUpperCase();
      if (url.includes("/read-state")) {
        if (method === "POST") {
          markReadCalls.push(url);
          return new Response(JSON.stringify({ last_read_at: justBefore11 }), {
            status: 200,
            headers: { "content-type": "application/json" },
          });
        }
        await gate;
        return new Response(JSON.stringify({ last_read_at: justBefore11 }), {
          status: 200,
          headers: { "content-type": "application/json" },
        });
      }
      return new Response("{}", { status: 404 });
    });

    try {
      render(
        <CollectionsProvider value={testCollections}>
          <LocaleProvider>
            <div style={{ width: 640, height: 420, display: "flex", flexDirection: "column" }}>
              <MessageList {...listProps("chan-delayed-unread", messages)} />
            </div>
          </LocaleProvider>
        </CollectionsProvider>,
      );

      await new Promise((resolve) => setTimeout(resolve, 60));
      expect(markReadCalls).toEqual([]);

      resolveReadState();

      await waitFor(() => {
        const scroller = document.querySelector<HTMLElement>('[data-testid="message-scroll"]');
        expect(scroller).toBeTruthy();
        const unreadRow = scroller!.querySelector<HTMLElement>('[data-message-id="d-11"]');
        expect(unreadRow).toBeTruthy();
        const sr = scroller!.getBoundingClientRect();
        const ur = unreadRow!.getBoundingClientRect();
        expect(ur.bottom).toBeGreaterThan(sr.top);
        expect(ur.top).toBeLessThan(sr.bottom);
      });
      await new Promise((resolve) => setTimeout(resolve, 60));
      expect(markReadCalls).toEqual([]);
    } finally {
      hasFocusSpy.mockRestore();
    }
  });

  it("drops stale offscreen measurements on live container width change and keeps scroll on currentUserId readOnly switch", async () => {
    const messages = Array.from({ length: 50 }, (_, index) =>
      makeMessage(`w-${index}`, index, 1, "chan-width"),
    ).map((m, idx) =>
      idx === 0
        ? { ...m, content: "Long wrapped first message ".repeat(16) }
        : { ...m, content: `Message ${idx} short` },
    );

    const view = render(
      <CollectionsProvider value={testCollections}>
        <LocaleProvider>
          <div
            data-testid="width-host"
            style={{ width: 390, height: 420, display: "flex", flexDirection: "column" }}
          >
            <MessageList key="chan-width" {...listProps("chan-width", messages)} />
          </div>
        </LocaleProvider>
      </CollectionsProvider>,
    );

    const scroller = await waitFor(() => {
      const el = document.querySelector<HTMLElement>('[data-testid="message-scroll"]');
      expect(el).toBeTruthy();
      const maxTop = Math.max(0, el!.scrollHeight - el!.clientHeight);
      expect(Math.abs(el!.scrollTop - maxTop)).toBeLessThanOrEqual(2);
      return el!;
    });

    // Scroll to top so row w-0 is measured at 390px, then scroll back to bottom.
    scroller.scrollTop = 0;
    fireEvent.scroll(scroller);
    await waitFor(() => expect(scroller.querySelector('[data-message-id="w-0"]')).toBeTruthy());

    // Archive/readOnly flips currentUserId to null while reading at scrollTop=0: must keep scrollTop!
    view.rerender(
      <CollectionsProvider value={testCollections}>
        <LocaleProvider>
          <div
            data-testid="width-host"
            style={{ width: 390, height: 420, display: "flex", flexDirection: "column" }}
          >
            <MessageList
              key="chan-width"
              {...listProps("chan-width", messages, { currentUserId: null })}
            />
          </div>
        </LocaleProvider>
      </CollectionsProvider>,
    );
    expect(scroller.scrollTop).toBeLessThan(100);

    // Return to bottom so w-0 is offscreen, restore alice, and resize host from 390px -> 800px.
    scroller.scrollTop = scroller.scrollHeight;
    fireEvent.scroll(scroller);
    await waitFor(() => expect(scroller.querySelector('[data-message-id="w-0"]')).toBeNull());

    view.rerender(
      <CollectionsProvider value={testCollections}>
        <LocaleProvider>
          <div
            data-testid="width-host"
            style={{ width: 800, height: 420, display: "flex", flexDirection: "column" }}
          >
            <MessageList key="chan-width" {...listProps("chan-width", messages)} />
          </div>
        </LocaleProvider>
      </CollectionsProvider>,
    );

    // Let ResizeObserver + animation frame deliver the new 800px measurements.
    await new Promise((resolve) =>
      requestAnimationFrame(() => requestAnimationFrame(() => resolve(undefined))),
    );
    await new Promise((resolve) => setTimeout(resolve, 40));

    // Switch channel to flush measurement cache snapshot.
    view.rerender(
      <CollectionsProvider value={testCollections}>
        <LocaleProvider>
          <div style={{ width: 800, height: 420, display: "flex", flexDirection: "column" }}>
            <MessageList key="chan-other" {...listProps("chan-other", messages.slice(0, 5))} />
          </div>
        </LocaleProvider>
      </CollectionsProvider>,
    );

    const cached = getChannelMeasurementCache(buildChannelScrollKey("chan-width", "alice"));
    expect(cached).toBeDefined();
    expect(cached!.containerWidth).toBeGreaterThan(700);
    // Offscreen row w-0 was only measured at 390px so it must NOT leak into the 800px cache.
    expect(cached!.sizes.has("w-0")).toBe(false);
    expect(cached!.sizes.has("w-49")).toBe(true);
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

  it("opens at bottom and follows new replies in ThreadPane when targetMessageId is not in replies", async () => {
    const root = makeMessage("root-parent", 0, 2, "chan-thread-parent");
    const replies = Array.from(
      { length: 40 },
      (_, index) =>
        ({
          ...makeMessage(`preply-${index}`, index + 1, 4, "chan-thread-parent"),
          parent_id: "root-parent",
        }) as FeedMessage,
    );

    const view = render(
      <CollectionsProvider value={testCollections}>
        <LocaleProvider>
          <div style={{ position: "relative", width: 800, height: 480, display: "flex" }}>
            <ThreadPane
              root={root}
              replies={replies}
              targetMessageId="root-parent"
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
        '[data-scroll-restoration-id="thread-scroll:root-parent"]',
      );
      expect(threadScroller).toBeTruthy();
      const maxTop = Math.max(0, threadScroller!.scrollHeight - threadScroller!.clientHeight);
      expect(maxTop).toBeGreaterThan(500);
      expect(Math.abs(threadScroller!.scrollTop - maxTop)).toBeLessThanOrEqual(2);
    });

    const withIncoming = [
      ...replies,
      {
        ...makeMessage("preply-40", 41, 4, "chan-thread-parent"),
        parent_id: "root-parent",
      } as FeedMessage,
    ];
    view.rerender(
      <CollectionsProvider value={testCollections}>
        <LocaleProvider>
          <div style={{ position: "relative", width: 800, height: 480, display: "flex" }}>
            <ThreadPane
              root={root}
              replies={withIncoming}
              targetMessageId="root-parent"
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
        '[data-scroll-restoration-id="thread-scroll:root-parent"]',
      )!;
      const newMax = Math.max(0, threadScroller.scrollHeight - threadScroller.clientHeight);
      expect(Math.abs(threadScroller.scrollTop - newMax)).toBeLessThanOrEqual(2);
    });
  });
});
