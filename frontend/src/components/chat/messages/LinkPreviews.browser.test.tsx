import "../../../styles.css";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vite-plus/test";
import { ApiError } from "../../../lib/api/client";
import { HomePostContent } from "../../home/HomePostContent";
import { LinkPreviews } from "./LinkPreviews.tsx";
import { DEFAULT_CONFIG } from "../../../lib/config";
import { http, HttpResponse } from "msw";
import { mockServer } from "../../../test/browser-setup";

const previewConfig = { ...DEFAULT_CONFIG, summalyUrl: "https://summaly.example/url" };
const api = vi.hoisted(() => ({ get: vi.fn() }));
vi.mock("../../../lib/summaly", () => ({ createSummalyClient: () => api }));
vi.mock("../../../lib/electric/collections-context", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../lib/electric/collections-context")>()),
  useOptionalCollections: () => ({
    config: previewConfig,
  }),
}));
afterEach(() => {
  api.get.mockReset();
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

it("renders metadata and loads the thumbnail directly from the runtime proxy endpoint", async () => {
  vi.spyOn(globalThis, "fetch").mockImplementation(
    async (input) =>
      new Response(
        input === "/summaly-endpoint" ? "https://summaly.example/url" : "https://media.example/",
        { headers: { "Content-Type": "text/plain" } },
      ),
  );
  mockServer.use(
    http.get("https://media.example/", () =>
      HttpResponse.arrayBuffer(
        Uint8Array.from(
          atob(
            "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==",
          ),
          (c) => c.charCodeAt(0),
        ).buffer,
        { headers: { "Content-Type": "image/png" } },
      ),
    ),
  );
  api.get.mockResolvedValue({
    title: "Example title",
    description: "Example description",
    image: "https://images.example/cover.png",
    siteName: "Example Blog",
    icon: "https://example.com/favicon.ico",
  });
  render(<LinkPreviews content="https://example.com/preview-one" />);
  const card = await screen.findByRole("link", { name: /Example title/ });
  expect(card.getAttribute("href")).toBe("https://example.com/preview-one");
  expect(screen.getByText("Example description")).toBeInTheDocument();
  expect(screen.getByText("Example Blog")).toBeInTheDocument();
  expect(screen.queryByText("example.com")).toBeNull();
  await waitFor(() =>
    expect(card.querySelector("img")?.src).toMatch(/^https:\/\/media.example\/\?url=/),
  );
  const img = card.querySelector("img")!;
  const favicon = card.querySelectorAll("img")[1];
  expect(new URL(favicon.src).searchParams.get("url")).toBe("https://example.com/favicon.ico");
  expect(favicon.nextElementSibling).toHaveTextContent("Example Blog");
  expect(new URL(img.src).searchParams.get("url")).toBe("https://images.example/cover.png");
  const initialBox = card.getBoundingClientRect();
  for (const [width, height] of [
    [1200, 630],
    [600, 600],
    [400, 800],
  ] as const) {
    Object.defineProperties(img, {
      naturalWidth: { configurable: true, value: width },
      naturalHeight: { configurable: true, value: height },
    });
    fireEvent.load(img);
    expect(card.getBoundingClientRect().height).toBe(initialBox.height);
    expect(getComputedStyle(card).flexDirection).toBe("row");
  }
  fireEvent.error(img);
  expect(card.querySelectorAll("img")).toHaveLength(1);
  fireEvent.error(favicon);
  expect(card.querySelector("img")).toBeNull();
  expect(card.getBoundingClientRect().height).toBe(initialBox.height);
  expect(card).toBeInTheDocument();
});

it("keeps a same-sized link fallback on failure and ignores stale responses after edits", async () => {
  let finish!: (data: unknown) => void;
  api.get
    .mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    )
    .mockRejectedValueOnce(new Error("unavailable"));
  const view = render(<LinkPreviews content="https://example.com/old-preview" />);
  await waitFor(() => expect(finish).toBeDefined());
  view.rerender(<LinkPreviews content="https://example.com/failed-preview" />);
  await waitFor(() => expect(finish).toBeDefined());
  finish({ title: "Old title", description: "", image: null });
  await waitFor(() =>
    expect(api.get).toHaveBeenCalledWith(
      expect.stringContaining("failed-preview"),
      expect.any(Object),
    ),
  );
  expect(screen.getByRole("link")).toBeInTheDocument();
  expect(screen.getByRole("link").getBoundingClientRect().height).toBe(128);
  expect(screen.queryByText("Old title")).toBeNull();
});

it("retries an upstream 422 while mounted and shares retries across views", async () => {
  vi.useFakeTimers();
  api.get
    .mockRejectedValueOnce(new ApiError(422, "error"))
    .mockResolvedValue({ title: "Recovered card", description: "", image: null });
  render(
    <>
      <LinkPreviews content="https://example.com/retry-mounted" />
      <LinkPreviews content="https://example.com/retry-mounted" />
    </>,
  );
  await act(async () => {
    await vi.advanceTimersByTimeAsync(0);
  });
  expect(api.get).toHaveBeenCalledTimes(1);
  await act(async () => {
    await vi.advanceTimersByTimeAsync(2_000);
  });
  expect(api.get).toHaveBeenCalledTimes(2);
  expect(screen.getAllByRole("link", { name: /Recovered card/ })).toHaveLength(2);
});

it("bounds transient retries and lets a later mount retry after exhaustion", async () => {
  vi.useFakeTimers();
  api.get.mockRejectedValue(new ApiError(429, "busy"));
  const view = render(<LinkPreviews content="https://example.com/retry-exhausted" />);
  await act(async () => {
    await vi.advanceTimersByTimeAsync(0);
  });
  await act(async () => {
    await vi.advanceTimersByTimeAsync(14_000);
  });
  expect(api.get).toHaveBeenCalledTimes(4);
  expect(screen.getByRole("link")).toBeInTheDocument();
  expect(screen.getByRole("link").getBoundingClientRect().height).toBe(128);
  await act(async () => {
    await vi.advanceTimersByTimeAsync(60_000);
  });
  expect(api.get).toHaveBeenCalledTimes(4);
  view.unmount();
  api.get.mockResolvedValue({ title: "Later card", description: "", image: null });
  render(<LinkPreviews content="https://example.com/retry-exhausted" />);
  await act(async () => {
    await vi.advanceTimersByTimeAsync(0);
  });
  expect(api.get).toHaveBeenCalledTimes(5);
  expect(screen.getByRole("link", { name: /Later card/ })).toBeInTheDocument();
});

it("does not retry permanent API failures", async () => {
  vi.useFakeTimers();
  api.get.mockRejectedValue(new ApiError(400, "unsupported URL"));
  render(<LinkPreviews content="https://example.com/permanent-failure" />);
  await act(async () => {
    await vi.advanceTimersByTimeAsync(60_000);
  });
  expect(api.get).toHaveBeenCalledTimes(1);
  expect(screen.getByRole("link")).toBeInTheDocument();
  expect(screen.getByRole("link").getBoundingClientRect().height).toBe(128);
});

it("resolves quoted renotes in Home and shares a pending lookup with chat", async () => {
  const url = "https://misskey.example/notes/home-quoted-original";
  let finish!: (data: unknown) => void;
  api.get.mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const view = render(
    <>
      <HomePostContent content={`**引用へのコメント**\n\n<${url}>`} />
      <HomePostContent content={`<${url}>`} />
      <LinkPreviews content={url} />
    </>,
  );
  await waitFor(() => expect(api.get).toHaveBeenCalledTimes(1));
  expect(api.get).toHaveBeenCalledWith(url, expect.any(Object));
  expect(screen.getByText("引用へのコメント")).toBeInTheDocument();
  // Removing one consumer must not cancel the other cards' shared request.
  view.rerender(
    <>
      <HomePostContent content={`<${url}>`} />
    </>,
  );
  expect(api.get.mock.calls[0][1].signal.aborted).toBe(false);
  finish({ title: "引用元の投稿", description: "引用元の本文", image: null });
  const card = await screen.findByRole("link", { name: /引用元の投稿/ });
  expect(card).toHaveAttribute("href", url);
  expect(screen.getByText("引用元の本文")).toBeInTheDocument();
  for (const width of [280, 960]) {
    view.container.style.width = `${width}px`;
    expect(card.getBoundingClientRect().width).toBeLessThanOrEqual(width);
    expect(card.scrollWidth).toBeLessThanOrEqual(card.clientWidth);
  }
  expect(api.get).toHaveBeenCalledTimes(1);
});

it("keeps the quote link when its Home preview cannot be resolved", async () => {
  const url = "https://misskey.example/notes/home-missing-quote";
  api.get.mockRejectedValue(new ApiError(404, "missing"));
  render(<HomePostContent content={`<${url}>`} />);
  await waitFor(() => expect(api.get).toHaveBeenCalledTimes(1));
  expect(screen.getAllByRole("link").every((link) => link.getAttribute("href") === url)).toBe(true);
  expect(screen.getByRole("link", { name: url.replace("https://", "") })).toBeInTheDocument();
});

it("defers Home requests until nearby, shares active views and cancels offscreen or hidden work", async () => {
  const callbacks: ((entries: { isIntersecting: boolean }[]) => void)[] = [];
  const disconnect = vi.fn();
  vi.stubGlobal(
    "IntersectionObserver",
    class {
      constructor(
        callback: (entries: { isIntersecting: boolean }[]) => void,
        options: IntersectionObserverInit,
      ) {
        expect(options.rootMargin).toBe("200px 0px");
        callbacks.push(callback);
      }
      observe() {}
      disconnect = disconnect;
    },
  );
  const visibility = vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
  const pending: { signal: AbortSignal; finish: (data: unknown) => void }[] = [];
  api.get.mockImplementation(
    (_url: string, { signal }: { signal: AbortSignal }) =>
      new Promise((resolve, reject) => {
        pending.push({ signal, finish: resolve });
        signal.addEventListener("abort", () => reject(signal.reason), { once: true });
      }),
  );
  const url = "https://example.com/deferred-home-preview";
  const view = render(
    <>
      <HomePostContent content={url} />
      <HomePostContent content={url} />
    </>,
  );
  await waitFor(() => expect(callbacks).toHaveLength(2));
  expect(api.get).not.toHaveBeenCalled();
  expect(screen.getAllByRole("link")).toHaveLength(2);
  await act(async () => callbacks[0]([{ isIntersecting: true }]));
  await waitFor(() => expect(api.get).toHaveBeenCalledTimes(1));
  expect(screen.getAllByRole("link")).toHaveLength(2);
  await act(async () => callbacks[1]([{ isIntersecting: true }]));
  expect(api.get).toHaveBeenCalledTimes(1);
  await act(async () => callbacks[0]([{ isIntersecting: false }]));
  expect(pending[0].signal.aborted).toBe(false);
  await act(async () => callbacks[1]([{ isIntersecting: false }]));
  expect(pending[0].signal.aborted).toBe(true);
  await act(async () => callbacks[0]([{ isIntersecting: true }]));
  await waitFor(() => expect(api.get).toHaveBeenCalledTimes(2));
  visibility.mockReturnValue("hidden");
  await act(async () => document.dispatchEvent(new Event("visibilitychange")));
  expect(pending[1].signal.aborted).toBe(true);
  visibility.mockReturnValue("visible");
  await act(async () => document.dispatchEvent(new Event("visibilitychange")));
  await waitFor(() => expect(api.get).toHaveBeenCalledTimes(3));
  await act(async () =>
    pending[2].finish({ title: "取得できたHomeカード", description: "説明", image: null }),
  );
  await waitFor(() =>
    expect(screen.getAllByRole("link", { name: /取得できたHomeカード/ })).toHaveLength(2),
  );
  await act(async () => callbacks[1]([{ isIntersecting: true }]));
  await waitFor(() =>
    expect(screen.getAllByRole("link", { name: /取得できたHomeカード/ })).toHaveLength(2),
  );
  expect(api.get).toHaveBeenCalledTimes(3);
  await act(async () => {
    callbacks[0]([{ isIntersecting: false }]);
    callbacks[1]([{ isIntersecting: false }]);
  });
  expect(screen.getAllByRole("link", { name: /取得できたHomeカード/ })).toHaveLength(2);
  view.unmount();
  expect(disconnect).toHaveBeenCalledTimes(2);
});

it("prefetches within 200px of the real scrolling pane and defers more distant posts", async () => {
  const near = "https://example.com/near-scroll-preview";
  const far = "https://example.com/far-scroll-preview";
  api.get.mockImplementation(async (url: string) => ({
    title: url === near ? "画面の100px下のカード" : "遠い投稿のカード",
    description: "説明",
    image: null,
  }));
  const view = render(
    <div style={{ height: 160, overflowY: "auto" }}>
      <div style={{ height: 260 }} />
      <HomePostContent content={near} />
      <div style={{ height: 1000 }} />
      <HomePostContent content={far} />
    </div>,
  );
  await screen.findByRole("link", { name: /画面の100px下のカード/ });
  expect(api.get).toHaveBeenCalledTimes(1);
  expect(api.get).toHaveBeenCalledWith(near, expect.any(Object));
  const scroller = view.container.firstElementChild!;
  scroller.scrollTop = scroller.scrollHeight;
  fireEvent.scroll(scroller);
  await screen.findByRole("link", { name: /遠い投稿のカード/ });
  expect(api.get).toHaveBeenCalledTimes(2);
});

it("shows the cached card during a slow background refresh and replaces every mounted copy", async () => {
  vi.useFakeTimers();
  const url = "https://example.com/stale-preview-card";
  api.get.mockResolvedValueOnce({
    title: "保存済みのカード",
    description: "前の説明",
    image: null,
  });
  const first = render(<LinkPreviews content={url} />);
  await act(async () => {
    await vi.advanceTimersByTimeAsync(0);
  });
  expect(screen.getByRole("link", { name: /保存済みのカード/ })).toBeInTheDocument();
  first.unmount();
  await act(async () => {
    await vi.advanceTimersByTimeAsync(300_001);
  });
  let finish!: (data: unknown) => void;
  api.get.mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  render(
    <>
      <LinkPreviews content={url} />
      <LinkPreviews content={url} />
    </>,
  );
  expect(screen.getAllByRole("link", { name: /保存済みのカード/ })).toHaveLength(2);
  await act(async () => {
    await vi.advanceTimersByTimeAsync(0);
  });
  expect(api.get).toHaveBeenCalledTimes(2);
  expect(screen.getAllByRole("link", { name: /保存済みのカード/ })).toHaveLength(2);
  await act(async () => {
    finish({ title: "更新済みのカード", description: "新しい説明", image: null });
  });
  expect(screen.getAllByRole("link", { name: /更新済みのカード/ })).toHaveLength(2);
  expect(screen.queryByText("保存済みのカード")).toBeNull();
});
