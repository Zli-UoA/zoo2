import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vite-plus/test";
import { ApiError } from "../../../lib/api/client";
import { LinkPreviews } from "./LinkPreviews.tsx";
import { DEFAULT_CONFIG } from "../../../lib/config";
import { http, HttpResponse } from "msw";
import { mockServer } from "../../../test/browser-setup";

const previewConfig = { ...DEFAULT_CONFIG, summalyUrl: "https://summaly.example/url" };
const api = vi.hoisted(() => ({ get: vi.fn() }));
vi.mock("../../../lib/summaly", () => ({ createSummalyClient: () => api }));
vi.mock("../../../lib/electric/collections-context", () => ({
  useOptionalCollections: () => ({
    config: previewConfig,
  }),
}));
afterEach(() => {
  api.get.mockReset();
  vi.useRealTimers();
  vi.restoreAllMocks();
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
  for (const [width, height, direction] of [
    [1200, 630, "flex-col"],
    [600, 600, "flex-row"],
    [400, 800, "flex-row"],
  ] as const) {
    Object.defineProperties(img, {
      naturalWidth: { configurable: true, value: width },
      naturalHeight: { configurable: true, value: height },
    });
    fireEvent.load(img);
    expect(card).toHaveClass(direction);
  }
  fireEvent.error(img);
  expect(card.querySelectorAll("img")).toHaveLength(1);
  fireEvent.error(favicon);
  expect(card.querySelector("img")).toBeNull();
  expect(card).toHaveClass("flex-row");
  expect(card).toBeInTheDocument();
});

it("hides failed cards and ignores stale responses after the URL is edited", async () => {
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
  expect(screen.queryByRole("link")).toBeNull();
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
  expect(screen.queryByRole("link")).toBeNull();
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
  expect(screen.queryByRole("link")).toBeNull();
});
