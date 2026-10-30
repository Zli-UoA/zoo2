import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vite-plus/test";
import { ApiError } from "../../../lib/api/client";
import { LinkPreviews } from "./LinkPreviews";
import { DEFAULT_CONFIG } from "../../../lib/config";

const api = vi.hoisted(() => ({ get: vi.fn() }));
vi.mock("../../../lib/api/client", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../lib/api/client")>()),
  createConfiguredApiClient: () => api,
}));
vi.mock("../../../lib/electric/collections-context", () => ({
  useOptionalCollections: () => ({ config: DEFAULT_CONFIG }),
}));
afterEach(() => {
  api.get.mockReset();
  vi.useRealTimers();
  vi.restoreAllMocks();
});

it("renders metadata and loads the thumbnail directly from the runtime proxy endpoint", async () => {
  vi.spyOn(globalThis, "fetch").mockResolvedValue(
    new Response("https://media.example/", { headers: { "Content-Type": "text/plain" } }),
  );
  api.get.mockResolvedValue({
    title: "Example title",
    description: "Example description",
    image: "https://images.example/cover.png",
  });
  render(<LinkPreviews content="https://example.com/preview-one" />);
  const card = await screen.findByRole("link", { name: /Example title/ });
  expect(card.getAttribute("href")).toBe("https://example.com/preview-one");
  expect(screen.getByText("Example description")).toBeInTheDocument();
  await waitFor(() =>
    expect(card.querySelector("img")?.src).toMatch(/^https:\/\/media.example\/\?url=/),
  );
  const img = card.querySelector("img")!;
  expect(new URL(img.src).searchParams.get("url")).toBe("https://images.example/cover.png");
  fireEvent.error(img);
  expect(card.querySelector("img")).toBeNull();
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
  view.rerender(<LinkPreviews content="https://example.com/failed-preview" />);
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

it("retries a rate-limited card while it stays mounted and shares retries across views", async () => {
  vi.useFakeTimers();
  api.get
    .mockRejectedValueOnce(new ApiError(429, "busy"))
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
