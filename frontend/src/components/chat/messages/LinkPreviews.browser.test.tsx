import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vite-plus/test";
import { LinkPreviews } from "./LinkPreviews";
import { DEFAULT_CONFIG } from "../../../lib/config";

const api = vi.hoisted(() => ({ get: vi.fn() }));
vi.mock("../../../lib/api/client", () => ({ createConfiguredApiClient: () => api }));
vi.mock("../../../lib/electric/collections-context", () => ({
  useOptionalCollections: () => ({ config: DEFAULT_CONFIG }),
}));
afterEach(() => vi.restoreAllMocks());

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
