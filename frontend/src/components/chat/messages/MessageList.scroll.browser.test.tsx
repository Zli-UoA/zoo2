import "../../../styles.css";

import {
  RouterProvider,
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  useNavigate,
  useRouterState,
} from "@tanstack/react-router";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";
import { useLayoutEffect, useRef } from "react";
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
import { requestMessageJump } from "./messageLink";
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

    async function runHistoryFlow(restoration: boolean | typeof shouldRestoreScroll) {
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
        scrollRestoration: restoration,
      });

      const view = render(
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

      // Scroll to a distinct mid-list offset and let newly mounted rows settle before navigating away.
      longScroller.scrollTop = 300;
      fireEvent.scroll(longScroller);
      await new Promise((resolve) => setTimeout(resolve, 100));
      await new Promise((resolve) =>
        requestAnimationFrame(() => requestAnimationFrame(() => resolve(undefined))),
      );
      fireEvent(longScroller, new Event("scrollend"));
      const midScrollTop = longScroller.scrollTop;
      expect(longMax - midScrollTop).toBeGreaterThan(500);

      router.history.back();
      await waitFor(() => {
        const el = document.querySelector<HTMLElement>('[data-testid="message-scroll"]');
        expect(el?.dataset.scrollRestorationId).toBe("message-scroll:short");
      });
      router.history.forward();
      await waitFor(() => {
        const el = document.querySelector<HTMLElement>('[data-testid="message-scroll"]');
        expect(el?.dataset.scrollRestorationId).toBe("message-scroll:long");
      });
      await new Promise((resolve) => setTimeout(resolve, 120));
      const afterForward = document.querySelector<HTMLElement>('[data-testid="message-scroll"]')!;
      const forwardMax = Math.max(0, afterForward.scrollHeight - afterForward.clientHeight);
      const topAfterForward = afterForward.scrollTop;
      view.unmount();
      return { midScrollTop, topAfterForward, forwardMax };
    }

    // Control: with scrollRestoration: true, TanStack Router restores the stale mid-list offset instead of landing at bottom.
    const withRestoration = await runHistoryFlow(true);
    expect(withRestoration.topAfterForward).toBeLessThan(800);
    expect(withRestoration.forwardMax - withRestoration.topAfterForward).toBeGreaterThan(500);
    expect(Math.abs(withRestoration.topAfterForward - withRestoration.midScrollTop)).toBeLessThan(
      250,
    );

    // Fix: with shouldRestoreScroll, /channels/* stays excluded and remains at bottom.
    const withoutRestoration = await runHistoryFlow(shouldRestoreScroll);
    expect(withoutRestoration.forwardMax).toBeGreaterThan(1000);
    expect(
      Math.abs(withoutRestoration.topAfterForward - withoutRestoration.forwardMax),
    ).toBeLessThanOrEqual(2);
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
    await new Promise((r) =>
      requestAnimationFrame(() => requestAnimationFrame(() => r(undefined))),
    );

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

  it("remounts an offscreen row after width change without reusing stale height before RO", async () => {
    const messages = Array.from({ length: 50 }, (_, index) =>
      makeMessage(`rw-${index}`, index, 1, "chan-width-remount"),
    ).map((m, idx) =>
      idx === 0
        ? { ...m, content: "Long wrapped first message ".repeat(16) }
        : { ...m, content: `Message ${idx} short` },
    );

    const view = render(
      <CollectionsProvider value={testCollections}>
        <LocaleProvider>
          <div style={{ width: 390, height: 420, display: "flex", flexDirection: "column" }}>
            <MessageList key="chan-width-remount" {...listProps("chan-width-remount", messages)} />
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

    scroller.scrollTop = 0;
    fireEvent.scroll(scroller);
    const narrowRow = await waitFor(() => {
      const r = scroller.querySelector<HTMLElement>('[data-message-id="rw-0"]');
      expect(r).toBeTruthy();
      return r!;
    });
    const narrowHeight = narrowRow.getBoundingClientRect().height;
    expect(narrowHeight).toBeGreaterThan(80);

    // Wait for ResizeObserver to commit narrowHeight into itemSizeCache (rw-1 shifts below rw-0)
    // before scrolling rw-0 offscreen, so itemSizeCache retains stale narrowHeight across width change.
    await waitFor(() => {
      const row0 = scroller.querySelector<HTMLElement>('[data-message-id="rw-0"]');
      const row1 = scroller.querySelector<HTMLElement>('[data-message-id="rw-1"]');
      expect(row0).toBeTruthy();
      expect(row1).toBeTruthy();
      const gap = row1!.getBoundingClientRect().top - row0!.getBoundingClientRect().top;
      expect(gap).toBeGreaterThanOrEqual(Math.floor(narrowHeight) - 2);
    });

    // Move back to bottom so rw-0 unmounts, then resize 390px -> 800px.
    scroller.scrollTop = scroller.scrollHeight;
    fireEvent.scroll(scroller);
    await waitFor(() => expect(scroller.querySelector('[data-message-id="rw-0"]')).toBeNull());

    view.rerender(
      <CollectionsProvider value={testCollections}>
        <LocaleProvider>
          <div style={{ width: 800, height: 420, display: "flex", flexDirection: "column" }}>
            <MessageList key="chan-width-remount" {...listProps("chan-width-remount", messages)} />
          </div>
        </LocaleProvider>
      </CollectionsProvider>,
    );
    await waitFor(() => expect(scroller.clientWidth).toBeGreaterThan(700));
    await new Promise((resolve) =>
      requestAnimationFrame(() => requestAnimationFrame(() => resolve(undefined))),
    );
    // Wait past @tanstack/virtual-core's default isScrollingResetDelay (150ms) so isScrolling is
    // deterministically false before remounting rw-0 synchronously (before ResizeObserver delivery).
    await new Promise((resolve) => setTimeout(resolve, 300));

    // Remount rw-0 synchronously during React commit (while isScrolling=false, before ResizeObserver),
    // then immediately switch channel to confirm !entry measured 800px instead of stale 390px cache.
    view.rerender(
      <CollectionsProvider value={testCollections}>
        <LocaleProvider>
          <div style={{ width: 800, height: 420, display: "flex", flexDirection: "column" }}>
            <MessageList
              key="chan-width-remount"
              {...listProps("chan-width-remount", [...messages.slice(0, 1), ...messages.slice(45)])}
            />
          </div>
        </LocaleProvider>
      </CollectionsProvider>,
    );
    expect(scroller.querySelector('[data-message-id="rw-0"]')).toBeTruthy();

    view.rerender(
      <CollectionsProvider value={testCollections}>
        <LocaleProvider>
          <div style={{ width: 800, height: 420, display: "flex", flexDirection: "column" }}>
            <MessageList
              key="chan-width-next"
              {...listProps("chan-width-next", messages.slice(0, 5))}
            />
          </div>
        </LocaleProvider>
      </CollectionsProvider>,
    );

    const cached = getChannelMeasurementCache(buildChannelScrollKey("chan-width-remount", "alice"));
    expect(cached).toBeDefined();
    expect(cached!.containerWidth).toBeGreaterThan(700);
    const savedWide = cached!.sizes.get("rw-0");
    expect(savedWide).toBeDefined();
    expect(savedWide!).toBeLessThan(narrowHeight - 20);
  });

  it("defers unread resolution while live feed is loading or initial read-state fails until resolved", async () => {
    const messages = Array.from({ length: 50 }, (_, index) =>
      makeMessage(`u-${index}`, index, 2, "chan-deferred"),
    );
    const before11 = new Date(Date.UTC(2026, 0, 1, 0, 10, 30)).toISOString();
    const markReadCalls: string[] = [];
    const hasFocusSpy = vi.spyOn(document, "hasFocus").mockReturnValue(true);
    let failGet = false;

    fetchSpy.mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url =
        typeof input === "string" ? input : input instanceof Request ? input.url : String(input);
      const method = (
        init?.method ?? (input instanceof Request ? input.method : "GET")
      ).toUpperCase();
      if (url.includes("/read-state")) {
        if (method === "POST") {
          markReadCalls.push(url);
          return new Response(JSON.stringify({ last_read_at: before11 }), {
            status: 200,
            headers: { "content-type": "application/json" },
          });
        }
        if (failGet) {
          return new Response("error", { status: 500 });
        }
        return new Response(JSON.stringify({ last_read_at: before11 }), {
          status: 200,
          headers: { "content-type": "application/json" },
        });
      }
      return new Response("{}", { status: 404 });
    });

    try {
      // (a) Cached rows displayed while isLoading=true and read-state GET finishes early.
      const view = render(
        <CollectionsProvider value={testCollections}>
          <LocaleProvider>
            <div style={{ width: 640, height: 420, display: "flex", flexDirection: "column" }}>
              <MessageList
                key="chan-deferred-a"
                {...listProps("chan-deferred-a", messages, { isLoading: true })}
              />
            </div>
          </LocaleProvider>
        </CollectionsProvider>,
      );

      await new Promise((resolve) => setTimeout(resolve, 60));
      expect(markReadCalls).toEqual([]);

      // Live rows arrive with identical 50 messages -> must jump to unread u-11 without marking read.
      view.rerender(
        <CollectionsProvider value={testCollections}>
          <LocaleProvider>
            <div style={{ width: 640, height: 420, display: "flex", flexDirection: "column" }}>
              <MessageList
                key="chan-deferred-a"
                {...listProps("chan-deferred-a", messages, { isLoading: false })}
              />
            </div>
          </LocaleProvider>
        </CollectionsProvider>,
      );

      await waitFor(() => {
        const scroller = document.querySelector<HTMLElement>('[data-testid="message-scroll"]')!;
        const unreadRow = scroller.querySelector<HTMLElement>('[data-message-id="u-11"]');
        expect(unreadRow).toBeTruthy();
        const sr = scroller.getBoundingClientRect();
        const ur = unreadRow!.getBoundingClientRect();
        expect(ur.bottom).toBeGreaterThan(sr.top);
        expect(ur.top).toBeLessThan(sr.bottom);
      });
      await new Promise((resolve) => setTimeout(resolve, 50));
      expect(markReadCalls).toEqual([]);

      // (b) Initial GET /read-state fails (500), then succeeds on window focus retry.
      failGet = true;
      view.rerender(
        <CollectionsProvider value={testCollections}>
          <LocaleProvider>
            <div style={{ width: 640, height: 420, display: "flex", flexDirection: "column" }}>
              <MessageList key="chan-deferred-b" {...listProps("chan-deferred-b", messages)} />
            </div>
          </LocaleProvider>
        </CollectionsProvider>,
      );

      await new Promise((resolve) => setTimeout(resolve, 60));
      expect(markReadCalls).toEqual([]);

      failGet = false;
      window.dispatchEvent(new Event("focus"));

      await waitFor(() => {
        const scroller = document.querySelector<HTMLElement>('[data-testid="message-scroll"]')!;
        const unreadRow = scroller.querySelector<HTMLElement>('[data-message-id="u-11"]');
        expect(unreadRow).toBeTruthy();
        const sr = scroller.getBoundingClientRect();
        const ur = unreadRow!.getBoundingClientRect();
        expect(ur.bottom).toBeGreaterThan(sr.top);
        expect(ur.top).toBeLessThan(sr.bottom);
      });
      await new Promise((resolve) => setTimeout(resolve, 50));
      expect(markReadCalls).toEqual([]);
    } finally {
      hasFocusSpy.mockRestore();
    }
  });

  it("re-aligns targetMessageId when a replace snapshot moves the target index", async () => {
    const targetMsg = makeMessage("target-shared", 45, 2, "chan-replace-target");
    const cachedMessages = Array.from({ length: 50 }, (_, index) =>
      index === 45 ? targetMsg : makeMessage(`old-${index}`, index, 2, "chan-replace-target"),
    );
    const liveMessages = [
      ...Array.from({ length: 6 }, (_, index) =>
        makeMessage(`live-pre-${index}`, index, 2, "chan-replace-target"),
      ),
      targetMsg,
      ...Array.from({ length: 45 }, (_, index) =>
        makeMessage(`live-post-${index}`, index + 7, 2, "chan-replace-target"),
      ),
    ];

    const view = render(
      <CollectionsProvider value={testCollections}>
        <LocaleProvider>
          <div style={{ width: 640, height: 420, display: "flex", flexDirection: "column" }}>
            <MessageList
              key="chan-replace-target"
              {...listProps("chan-replace-target", cachedMessages, {
                targetMessageId: "target-shared",
              })}
            />
          </div>
        </LocaleProvider>
      </CollectionsProvider>,
    );

    await waitFor(() => {
      const scroller = document.querySelector<HTMLElement>('[data-testid="message-scroll"]')!;
      const row = scroller.querySelector<HTMLElement>('[data-message-id="target-shared"]');
      expect(row).toBeTruthy();
      const sr = scroller.getBoundingClientRect();
      const tr = row!.getBoundingClientRect();
      expect(tr.top).toBeGreaterThanOrEqual(sr.top - 4);
      expect(tr.bottom).toBeLessThanOrEqual(sr.bottom + 4);
    });

    // Replace both ends of the list and shift target-shared from index 45 to index 6.
    view.rerender(
      <CollectionsProvider value={testCollections}>
        <LocaleProvider>
          <div style={{ width: 640, height: 420, display: "flex", flexDirection: "column" }}>
            <MessageList
              key="chan-replace-target"
              {...listProps("chan-replace-target", liveMessages, {
                targetMessageId: "target-shared",
              })}
            />
          </div>
        </LocaleProvider>
      </CollectionsProvider>,
    );

    await waitFor(() => {
      const scroller = document.querySelector<HTMLElement>('[data-testid="message-scroll"]')!;
      const row = scroller.querySelector<HTMLElement>('[data-message-id="target-shared"]');
      expect(row).toBeTruthy();
      const sr = scroller.getBoundingClientRect();
      const tr = row!.getBoundingClientRect();
      expect(tr.top).toBeGreaterThanOrEqual(sr.top - 4);
      expect(tr.bottom).toBeLessThanOrEqual(sr.bottom + 4);
    });
  });

  it("scrolls back to the target when a link to the already-targeted message is pressed again", async () => {
    const channelId = "chan-jump";
    const link = `${window.location.origin}/channels/${channelId}?message=j-8`;
    const messages = Array.from({ length: 50 }, (_, index) => ({
      ...makeMessage(`j-${index}`, index, 2, channelId),
      // 末尾に近い投稿が、URL がすでに指している j-8 へのリンクを含む。
      ...(index === 47 ? { content: `詳細は ${link} を見てください` } : {}),
    }));

    function Channel() {
      const location = useRouterState({ select: (state) => state.location });
      const { message } = location.search as { message?: string };
      return (
        <CollectionsProvider value={testCollections}>
          <LocaleProvider>
            <div style={{ width: 640, height: 420, display: "flex", flexDirection: "column" }}>
              <MessageList
                key={channelId}
                {...listProps(channelId, messages, { targetMessageId: message })}
              />
            </div>
          </LocaleProvider>
        </CollectionsProvider>
      );
    }
    const root = createRootRoute();
    const route = createRoute({
      getParentRoute: () => root,
      path: "/channels/$channelId",
      validateSearch: (search: Record<string, unknown>): { message?: string } => ({
        message: typeof search.message === "string" ? search.message : undefined,
      }),
      component: Channel,
    });
    const router = createRouter({
      routeTree: root.addChildren([route]),
      history: createMemoryHistory({ initialEntries: [`/channels/${channelId}?message=j-8`] }),
    });
    render(<RouterProvider router={router} />);

    const scroller = () => document.querySelector<HTMLElement>('[data-testid="message-scroll"]')!;
    const targetInView = () => {
      const row = scroller().querySelector<HTMLElement>('[data-message-id="j-8"]');
      if (!row) return false;
      const sr = scroller().getBoundingClientRect();
      const tr = row.getBoundingClientRect();
      return tr.top >= sr.top - 4 && tr.bottom <= sr.bottom + 4;
    };
    const bodyLink = () =>
      scroller().querySelector<HTMLAnchorElement>(`a[href="/channels/${channelId}?message=j-8"]`);

    await waitFor(() => expect(targetInView()).toBe(true));
    // 対象の行へスクロールしたら、その行だけを強調する（常時の枠線は付けない）。
    const targetOverlay = () =>
      scroller().querySelector<HTMLElement>(
        '[data-message-id="j-8"] [data-message-target-highlight]',
      );
    await waitFor(() => expect(targetOverlay()).toBeTruthy());
    expect(scroller().querySelector('[data-message-id="j-8"]')!.className).not.toContain("ring");
    expect(scroller().querySelectorAll("[data-message-target-highlight]")).toHaveLength(1);
    const firstOverlay = targetOverlay()!;

    // 読み進めて、対象から離れる。URL は `?message=j-8` のまま。
    scroller().scrollTop = scroller().scrollHeight;
    await waitFor(() => {
      expect(targetInView()).toBe(false);
      expect(bodyLink()).toBeTruthy();
    });
    expect(router.state.location.href).toBe(`/channels/${channelId}?message=j-8`);

    // 修飾キーつきのクリックは、ブラウザーに任せる。この画面は動かさない（実際の遷移は止める）。
    const stopNavigation = (event: MouseEvent) => event.preventDefault();
    document.addEventListener("click", stopNavigation);
    try {
      fireEvent.click(bodyLink()!, { ctrlKey: true });
      await new Promise((resolve) => setTimeout(resolve, 200));
      expect(targetInView()).toBe(false);
    } finally {
      document.removeEventListener("click", stopNavigation);
    }

    // 普通のクリックで、同じ場所への移動になっても、対象の行へ戻り、強調を最初から再生し直す。
    fireEvent.click(bodyLink()!);
    await waitFor(() => expect(targetInView()).toBe(true));
    await waitFor(() => {
      expect(targetOverlay()).toBeTruthy();
      expect(targetOverlay()).not.toBe(firstOverlay);
    });
    expect(router.state.location.href).toBe(`/channels/${channelId}?message=j-8`);
  });

  it("highlights the target reply in ThreadPane once and replays it when the link is pressed again", async () => {
    const root = makeMessage("root-h", 0, 2, "chan-thread-h");
    const replies = Array.from(
      { length: 40 },
      (_, index) =>
        ({
          ...makeMessage(`reply-${index}`, index + 1, 4, "chan-thread-h"),
          parent_id: "root-h",
        }) as FeedMessage,
    );
    render(
      <CollectionsProvider value={testCollections}>
        <LocaleProvider>
          <div style={{ position: "relative", width: 800, height: 480, display: "flex" }}>
            <ThreadPane
              root={root}
              replies={replies}
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
    const scroller = () =>
      document.querySelector<HTMLElement>('[data-scroll-restoration-id="thread-scroll:root-h"]')!;
    const row = () => scroller().querySelector<HTMLElement>('[data-message-id="reply-3"]')!;
    const overlay = () =>
      row().querySelector<HTMLElement>("[data-message-target-highlight]") ?? undefined;
    const inView = () => {
      const sr = scroller().getBoundingClientRect();
      const tr = row().getBoundingClientRect();
      return tr.top >= sr.top - 4 && tr.bottom <= sr.bottom + 4;
    };

    // 対象の返信へスクロールしたら、その行だけを強調する。常時の枠線は付けない。
    await waitFor(() => {
      expect(inView()).toBe(true);
      expect(overlay()).toBeTruthy();
    });
    expect(row().className).not.toContain("ring");
    expect(scroller().querySelectorAll("[data-message-target-highlight]")).toHaveLength(1);
    const first = overlay()!;

    // 離れてから、すでに表示中の対象へのリンクをもう一度押すと、戻って最初から強調し直す。
    scroller().scrollTop = scroller().scrollHeight;
    await waitFor(() => expect(inView()).toBe(false));
    requestMessageJump("reply-3");
    await waitFor(() => {
      expect(inView()).toBe(true);
      expect(overlay()).toBeTruthy();
      expect(overlay()).not.toBe(first);
    });
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

  it("keeps following new appends after initial unread boundary lands near bottom", async () => {
    const messages = Array.from({ length: 20 }, (_, index) =>
      makeMessage(`tu-${index}`, index, 6, "chan-unread-tail"),
    );
    const beforeLast = new Date(Date.UTC(2026, 0, 1, 0, 18, 30)).toISOString();
    const hasFocusSpy = vi.spyOn(document, "hasFocus").mockReturnValue(true);

    fetchSpy.mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url =
        typeof input === "string" ? input : input instanceof Request ? input.url : String(input);
      const method = (
        init?.method ?? (input instanceof Request ? input.method : "GET")
      ).toUpperCase();
      if (url.includes("/read-state")) {
        if (method === "GET") {
          await Promise.resolve();
        }
        return new Response(JSON.stringify({ last_read_at: beforeLast }), {
          status: 200,
          headers: { "content-type": "application/json" },
        });
      }
      return new Response("{}", { status: 404 });
    });

    try {
      const view = render(
        <CollectionsProvider value={testCollections}>
          <LocaleProvider>
            <div style={{ width: 640, height: 420, display: "flex", flexDirection: "column" }}>
              <MessageList key="chan-unread-tail" {...listProps("chan-unread-tail", messages)} />
            </div>
          </LocaleProvider>
        </CollectionsProvider>,
      );

      await waitFor(() => {
        const scroller = document.querySelector<HTMLElement>('[data-testid="message-scroll"]');
        expect(scroller).toBeTruthy();
        expect(scroller!.querySelector('[data-message-id="tu-19"]')).toBeTruthy();
        const maxTop = Math.max(0, scroller!.scrollHeight - scroller!.clientHeight);
        expect(maxTop).toBeGreaterThan(800);
        expect(Math.abs(scroller!.scrollTop - maxTop)).toBeLessThanOrEqual(8);
      });
      await new Promise((resolve) =>
        requestAnimationFrame(() => requestAnimationFrame(() => resolve(undefined))),
      );

      const withIncoming = [
        ...messages,
        makeMessage("tu-21", 21, 6, "chan-unread-tail"), // odd index -> bob
      ];
      view.rerender(
        <CollectionsProvider value={testCollections}>
          <LocaleProvider>
            <div style={{ width: 640, height: 420, display: "flex", flexDirection: "column" }}>
              <MessageList
                key="chan-unread-tail"
                {...listProps("chan-unread-tail", withIncoming)}
              />
            </div>
          </LocaleProvider>
        </CollectionsProvider>,
      );

      await waitFor(() => {
        const scroller = document.querySelector<HTMLElement>('[data-testid="message-scroll"]')!;
        expect(scroller.querySelector('[data-message-id="tu-21"]')).toBeTruthy();
        const newMax = Math.max(0, scroller.scrollHeight - scroller.clientHeight);
        expect(Math.abs(scroller.scrollTop - newMax)).toBeLessThanOrEqual(2);
      });
    } finally {
      hasFocusSpy.mockRestore();
    }
  });

  it("stays at bottom or preserves reading offset on replace after initial unread resolution even if unreadBoundary remains", async () => {
    const messages = Array.from({ length: 40 }, (_, index) =>
      makeMessage(`rep-${index}`, index, 4, "chan-replace-bottom"),
    );
    const before10 = new Date(Date.UTC(2026, 0, 1, 0, 9, 30)).toISOString();
    const hasFocusSpy = vi.spyOn(document, "hasFocus").mockReturnValue(false);

    fetchSpy.mockImplementation(async (input: RequestInfo | URL) => {
      const url =
        typeof input === "string" ? input : input instanceof Request ? input.url : String(input);
      if (url.includes("/read-state")) {
        return new Response(JSON.stringify({ last_read_at: before10 }), {
          status: 200,
          headers: { "content-type": "application/json" },
        });
      }
      return new Response("{}", { status: 404 });
    });

    try {
      const view = render(
        <CollectionsProvider value={testCollections}>
          <LocaleProvider>
            <div style={{ width: 640, height: 420, display: "flex", flexDirection: "column" }}>
              <MessageList
                key="chan-replace-bottom"
                {...listProps("chan-replace-bottom", messages)}
              />
            </div>
          </LocaleProvider>
        </CollectionsProvider>,
      );

      await waitFor(() => {
        const scroller = document.querySelector<HTMLElement>('[data-testid="message-scroll"]');
        const row = scroller?.querySelector<HTMLElement>('[data-message-id="rep-10"]');
        expect(row).toBeTruthy();
        const sr = scroller!.getBoundingClientRect();
        const rr = row!.getBoundingClientRect();
        expect(rr.bottom).toBeGreaterThan(sr.top);
        expect(rr.top).toBeLessThan(sr.bottom);
      });

      const scroller = document.querySelector<HTMLElement>('[data-testid="message-scroll"]')!;
      scroller.scrollTop = scroller.scrollHeight;
      fireEvent.scroll(scroller);
      fireEvent(scroller, new Event("scrollend"));

      await waitFor(() => {
        expect(scroller.querySelector('[data-message-id="rep-39"]')).toBeTruthy();
        const maxTop = Math.max(0, scroller.scrollHeight - scroller.clientHeight);
        expect(Math.abs(scroller.scrollTop - maxTop)).toBeLessThanOrEqual(2);
      });

      // Replace window (oldest prepend + newest delete) while reading at bottom -> stay at bottom.
      const bottomReplaced = [
        makeMessage("rep-pre", -1, 4, "chan-replace-bottom"),
        ...messages.slice(0, 39),
      ];
      view.rerender(
        <CollectionsProvider value={testCollections}>
          <LocaleProvider>
            <div style={{ width: 640, height: 420, display: "flex", flexDirection: "column" }}>
              <MessageList
                key="chan-replace-bottom"
                {...listProps("chan-replace-bottom", bottomReplaced)}
              />
            </div>
          </LocaleProvider>
        </CollectionsProvider>,
      );

      await waitFor(() => {
        expect(scroller.querySelector('[data-message-id="rep-38"]')).toBeTruthy();
        const newMax = Math.max(0, scroller.scrollHeight - scroller.clientHeight);
        expect(Math.abs(scroller.scrollTop - newMax)).toBeLessThanOrEqual(2);
      });

      // Scroll up to rep-20, then replace again -> preserve reading anchor near rep-20 (not rep-10 or bottom).
      scroller.scrollTop = Math.max(0, Math.round(scroller.scrollHeight * 0.5));
      fireEvent.scroll(scroller);
      fireEvent(scroller, new Event("scrollend"));
      const anchorId = await waitFor(() => {
        const rows = Array.from(scroller.querySelectorAll<HTMLElement>("[data-message-id]"));
        const sr = scroller.getBoundingClientRect();
        const visible = rows.find((r) => {
          const rect = r.getBoundingClientRect();
          return rect.top >= sr.top && rect.bottom <= sr.bottom;
        });
        expect(visible).toBeTruthy();
        return visible!.dataset.messageId!;
      });
      const offsetBefore =
        scroller
          .querySelector<HTMLElement>(`[data-message-id="${anchorId}"]`)!
          .getBoundingClientRect().top - scroller.getBoundingClientRect().top;

      const midReplaced = [
        makeMessage("rep-pre2", -2, 4, "chan-replace-bottom"),
        ...bottomReplaced.slice(0, 39),
      ];
      view.rerender(
        <CollectionsProvider value={testCollections}>
          <LocaleProvider>
            <div style={{ width: 640, height: 420, display: "flex", flexDirection: "column" }}>
              <MessageList
                key="chan-replace-bottom"
                {...listProps("chan-replace-bottom", midReplaced)}
              />
            </div>
          </LocaleProvider>
        </CollectionsProvider>,
      );

      await new Promise((resolve) =>
        requestAnimationFrame(() => requestAnimationFrame(() => resolve(undefined))),
      );
      await waitFor(() => {
        const anchorAfter = scroller.querySelector<HTMLElement>(`[data-message-id="${anchorId}"]`);
        expect(anchorAfter).toBeTruthy();
        const offsetAfter =
          anchorAfter!.getBoundingClientRect().top - scroller.getBoundingClientRect().top;
        expect(Math.abs(offsetAfter - offsetBefore)).toBeLessThanOrEqual(25);
      });
    } finally {
      hasFocusSpy.mockRestore();
    }
  });

  it("preserves mid-scroll position without resetting estimated row height on live resize", async () => {
    const tall = Array.from({ length: 50 }, (_, index) =>
      makeMessage(`lr-${index}`, index, 7, "chan-live-resize"),
    );

    const view = render(
      <CollectionsProvider value={testCollections}>
        <LocaleProvider>
          <div style={{ width: 390, height: 420, display: "flex", flexDirection: "column" }}>
            <MessageList key="chan-live-resize" {...listProps("chan-live-resize", tall)} />
          </div>
        </LocaleProvider>
      </CollectionsProvider>,
    );

    await waitFor(() => {
      const scroller = document.querySelector<HTMLElement>('[data-testid="message-scroll"]');
      expect(scroller?.querySelector('[data-message-id="lr-49"]')).toBeTruthy();
    });

    // Switch away to cache the ~180px mean row height for bottom rows only (leaving top rows uncached).
    view.rerender(
      <CollectionsProvider value={testCollections}>
        <LocaleProvider>
          <div style={{ width: 390, height: 420, display: "flex", flexDirection: "column" }}>
            <MessageList
              key="chan-live-other"
              {...listProps("chan-live-other", tall.slice(0, 5))}
            />
          </div>
        </LocaleProvider>
      </CollectionsProvider>,
    );

    // Remount chan-live-resize from cache, then scroll slightly up while rows 0..30 remain unmounted/unmeasured.
    view.rerender(
      <CollectionsProvider value={testCollections}>
        <LocaleProvider>
          <div style={{ width: 390, height: 420, display: "flex", flexDirection: "column" }}>
            <MessageList key="chan-live-resize" {...listProps("chan-live-resize", tall)} />
          </div>
        </LocaleProvider>
      </CollectionsProvider>,
    );

    const scroller = await waitFor(() => {
      const el = document.querySelector<HTMLElement>('[data-testid="message-scroll"]');
      expect(el?.querySelector('[data-message-id="lr-49"]')).toBeTruthy();
      return el!;
    });

    const maxTop = Math.max(0, scroller.scrollHeight - scroller.clientHeight);
    scroller.scrollTop = Math.max(0, maxTop - 700);
    fireEvent.scroll(scroller);
    await new Promise((resolve) =>
      requestAnimationFrame(() => requestAnimationFrame(() => resolve(undefined))),
    );
    fireEvent(scroller, new Event("scrollend"));
    const scrollTopBeforeResize = scroller.scrollTop;

    // Live container width change >2px must NOT reset activeEstimateRef.rowHeight to 64px.
    view.rerender(
      <CollectionsProvider value={testCollections}>
        <LocaleProvider>
          <div style={{ width: 415, height: 420, display: "flex", flexDirection: "column" }}>
            <MessageList key="chan-live-resize" {...listProps("chan-live-resize", tall)} />
          </div>
        </LocaleProvider>
      </CollectionsProvider>,
    );
    await new Promise((resolve) =>
      requestAnimationFrame(() => requestAnimationFrame(() => resolve(undefined))),
    );

    // Scroll up 160px to trigger measurement & recalculation of virtualizer offsets; must not jump hundreds of px.
    scroller.scrollTop = Math.max(0, scrollTopBeforeResize - 160);
    fireEvent.scroll(scroller);
    await new Promise((resolve) =>
      requestAnimationFrame(() => requestAnimationFrame(() => resolve(undefined))),
    );
    fireEvent(scroller, new Event("scrollend"));
    expect(Math.abs(scroller.scrollTop - (scrollTopBeforeResize - 160))).toBeLessThanOrEqual(80);
  });

  it("retries targetMessageId alignment when rapid items reference churn cancels initial frame", async () => {
    const messages = Array.from({ length: 50 }, (_, index) =>
      makeMessage(`tc-${index}`, index, 3, "chan-target-retry"),
    );

    const view = render(
      <CollectionsProvider value={testCollections}>
        <LocaleProvider>
          <div style={{ width: 640, height: 420, display: "flex", flexDirection: "column" }}>
            <MessageList key="chan-target-retry" {...listProps("chan-target-retry", messages)} />
          </div>
        </LocaleProvider>
      </CollectionsProvider>,
    );

    const scroller = await waitFor(() => {
      const el = document.querySelector<HTMLElement>('[data-testid="message-scroll"]');
      expect(el).toBeTruthy();
      const maxTop = Math.max(0, el!.scrollHeight - el!.clientHeight);
      expect(maxTop).toBeGreaterThan(800);
      expect(Math.abs(el!.scrollTop - maxTop)).toBeLessThanOrEqual(2);
      return el!;
    });

    // Set targetMessageId and immediately churn items array references before the first rAF executes.
    for (let pass = 0; pass < 3; pass++) {
      view.rerender(
        <CollectionsProvider value={testCollections}>
          <LocaleProvider>
            <div style={{ width: 640, height: 420, display: "flex", flexDirection: "column" }}>
              <MessageList
                key="chan-target-retry"
                {...listProps(
                  "chan-target-retry",
                  messages.map((m) => ({ ...m })),
                  { targetMessageId: "tc-10" },
                )}
              />
            </div>
          </LocaleProvider>
        </CollectionsProvider>,
      );
    }

    await waitFor(() => {
      const targetRow = scroller.querySelector<HTMLElement>('[data-message-id="tc-10"]');
      expect(targetRow).toBeTruthy();
      const sr = scroller.getBoundingClientRect();
      const tr = targetRow!.getBoundingClientRect();
      const viewportMid = (sr.top + sr.bottom) / 2;
      const targetMid = (tr.top + tr.bottom) / 2;
      expect(Math.abs(targetMid - viewportMid)).toBeLessThan(sr.height * 0.35);
    });
  });

  it("blocks loadOlder and markRead during initial settling even if scroll events fire at top or bottom", async () => {
    function ScrollTriggerDuringInitialCommit({ onMountCommit }: { onMountCommit: () => void }) {
      const firedRef = useRef(false);
      useLayoutEffect(() => {
        if (firedRef.current) return;
        firedRef.current = true;
        onMountCommit();
      }, [onMountCommit]);
      return null;
    }

    const messages = Array.from({ length: 30 }, (_, index) =>
      makeMessage(`sg-${index}`, index, 7, "chan-settling-guard"),
    );
    const onLoadOlder = vi.fn();
    let firedLoadOlderCheck = false;

    // Case 1: scrollTop=0 scroll event fired during Commit 1 (while settlingRef.current=true) must NOT trigger loadOlder.
    const view1 = render(
      <CollectionsProvider value={testCollections}>
        <LocaleProvider>
          <div style={{ width: 640, height: 420, display: "flex", flexDirection: "column" }}>
            <MessageList
              key="chan-settling-loadolder"
              {...listProps("chan-settling-loadolder", messages, {
                hasOlder: true,
                onLoadOlder,
              })}
            />
            <ScrollTriggerDuringInitialCommit
              onMountCommit={() => {
                firedLoadOlderCheck = true;
                const scroller = document.querySelector<HTMLElement>(
                  '[data-testid="message-scroll"]',
                )!;
                scroller.scrollTop = 0;
                fireEvent.scroll(scroller);
              }}
            />
          </div>
        </LocaleProvider>
      </CollectionsProvider>,
    );

    expect(firedLoadOlderCheck).toBe(true);
    expect(onLoadOlder).not.toHaveBeenCalled();

    const scroller1 = document.querySelector<HTMLElement>('[data-testid="message-scroll"]')!;
    await waitFor(() => {
      const maxTop = Math.max(0, scroller1.scrollHeight - scroller1.clientHeight);
      expect(maxTop).toBeGreaterThan(500);
      expect(Math.abs(scroller1.scrollTop - maxTop)).toBeLessThanOrEqual(2);
    });
    await new Promise((resolve) =>
      requestAnimationFrame(() => requestAnimationFrame(() => resolve(undefined))),
    );
    expect(onLoadOlder).not.toHaveBeenCalled();

    // After settling completes, scrolling to top DOES invoke loadOlder.
    scroller1.scrollTop = 0;
    fireEvent.scroll(scroller1);
    expect(onLoadOlder).toHaveBeenCalledTimes(1);
    view1.unmount();

    // Case 2: scrollHeight scroll event fired during Commit 1 on unread channel must NOT call markRead.
    const markReadCalls: string[] = [];
    let firedMarkReadCheck = false;
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
        return new Response(JSON.stringify({ last_read_at: "2026-01-01T00:00:00Z" }), {
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
              <MessageList
                key="chan-settling-markread"
                {...listProps("chan-settling-markread", messages, {
                  unreadBoundaryId: "sg-5",
                })}
              />
              <ScrollTriggerDuringInitialCommit
                onMountCommit={() => {
                  firedMarkReadCheck = true;
                  const scroller = document.querySelector<HTMLElement>(
                    '[data-testid="message-scroll"]',
                  )!;
                  scroller.scrollTop = scroller.scrollHeight;
                  fireEvent.scroll(scroller);
                }}
              />
            </div>
          </LocaleProvider>
        </CollectionsProvider>,
      );

      expect(firedMarkReadCheck).toBe(true);
      expect(markReadCalls).toEqual([]);

      const scroller2 = document.querySelector<HTMLElement>('[data-testid="message-scroll"]')!;
      expect(scroller2.scrollHeight).toBeGreaterThan(scroller2.clientHeight + 100);
      await waitFor(() => {
        const unreadRow = scroller2.querySelector<HTMLElement>('[data-message-id="sg-5"]');
        expect(unreadRow).toBeTruthy();
        const sr = scroller2.getBoundingClientRect();
        const ur = unreadRow!.getBoundingClientRect();
        expect(ur.bottom).toBeGreaterThan(sr.top);
        expect(ur.top).toBeLessThan(sr.bottom);
      });
      await new Promise((resolve) => setTimeout(resolve, 50));
      expect(markReadCalls).toEqual([]);
    } finally {
      hasFocusSpy.mockRestore();
    }
  });

  it("isolates cached row measurements per user on the same channel and skips saving after logout cache clear", async () => {
    const messages = Array.from({ length: 20 }, (_, index) =>
      makeMessage(`ui-${index}`, index, 3, "chan-user-iso"),
    );

    const view = render(
      <CollectionsProvider value={testCollections}>
        <LocaleProvider>
          <div style={{ width: 640, height: 420, display: "flex", flexDirection: "column" }}>
            <MessageList
              key="chan-user-iso:alice"
              {...listProps("chan-user-iso", messages, { currentUserId: "alice" })}
            />
          </div>
        </LocaleProvider>
      </CollectionsProvider>,
    );

    await waitFor(() => {
      const scroller = document.querySelector<HTMLElement>('[data-testid="message-scroll"]');
      expect(scroller?.querySelector('[data-message-id="ui-19"]')).toBeTruthy();
    });

    // Leave channel as alice -> saves only under alice's key.
    view.rerender(
      <CollectionsProvider value={testCollections}>
        <LocaleProvider>
          <div style={{ width: 640, height: 420, display: "flex", flexDirection: "column" }}>
            <MessageList
              key="chan-user-other"
              {...listProps("chan-user-other", messages.slice(0, 5), { currentUserId: "alice" })}
            />
          </div>
        </LocaleProvider>
      </CollectionsProvider>,
    );

    const aliceKey = buildChannelScrollKey("chan-user-iso", "alice");
    const bobKey = buildChannelScrollKey("chan-user-iso", "bob");
    expect(getChannelMeasurementCache(aliceKey)).toBeDefined();
    expect(getChannelMeasurementCache(bobKey)).toBeUndefined();

    // Open same channel as bob, clear measurement cache (simulating logout), then unmount.
    view.rerender(
      <CollectionsProvider value={testCollections}>
        <LocaleProvider>
          <div style={{ width: 640, height: 420, display: "flex", flexDirection: "column" }}>
            <MessageList
              key="chan-user-iso:bob"
              {...listProps("chan-user-iso", messages, { currentUserId: "bob" })}
            />
          </div>
        </LocaleProvider>
      </CollectionsProvider>,
    );

    await waitFor(() => {
      const scroller = document.querySelector<HTMLElement>('[data-testid="message-scroll"]');
      expect(scroller?.querySelector('[data-message-id="ui-19"]')).toBeTruthy();
    });

    clearChannelMeasurementCache();
    view.unmount();

    expect(getChannelMeasurementCache(aliceKey)).toBeUndefined();
    expect(getChannelMeasurementCache(bobKey)).toBeUndefined();
  });
});
