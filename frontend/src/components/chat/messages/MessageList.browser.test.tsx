import "../../../styles.css";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vite-plus/test";
import { http, HttpResponse } from "msw";

import { DEFAULT_CONFIG } from "../../../lib/config";
import type { LinkPreviewData } from "../../../lib/summaly";
import { mockServer, startMockServer } from "../../../test/browser-setup";
import { LocaleProvider } from "../../i18n/LocaleProvider";
import { MessageList } from "./MessageList";

const config = { ...DEFAULT_CONFIG, summalyUrl: "https://scroll.example/summaly" };
const api = vi.hoisted(() => ({ get: vi.fn() }));
vi.mock("../../../lib/summaly", () => ({ createSummalyClient: () => api }));
vi.mock("../../../lib/electric/collections-context", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../lib/electric/collections-context")>()),
  useOptionalCollections: () => ({ config }),
}));

afterEach(() => api.get.mockReset());

const nextFrame = () =>
  new Promise<void>((resolve) =>
    requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
  );

it.each(["bottom", "returned", "visible", "above"] as const)(
  "preserves the reading position when a URL card expands (%s)",
  async (position) => {
    await startMockServer();
    let resolveMetadata!: (data: LinkPreviewData) => void;
    const metadata = new Promise<LinkPreviewData>((resolve) => {
      resolveMetadata = resolve;
    });
    let resolveImage!: () => void;
    const image = new Promise<void>((resolve) => {
      resolveImage = resolve;
    });
    api.get.mockReturnValue(metadata);
    mockServer.use(
      http.get("*/summaly-endpoint", () => HttpResponse.text(config.summalyUrl)),
      http.get("*/media-proxy-endpoint", () => HttpResponse.text("https://scroll.example/media")),
      http.get("https://scroll.example/media", async () => {
        await image;
        return HttpResponse.text(
          '<svg xmlns="http://www.w3.org/2000/svg" width="1200" height="630"/>',
          { headers: { "Content-Type": "image/svg+xml" } },
        );
      }),
    );
    const followsBottom = position === "bottom" || position === "returned";
    const previewIndex = followsBottom ? 19 : 1;
    const noop = () => {};
    render(
      <LocaleProvider>
        <div className="flex h-[400px] w-[360px] flex-col">
          <MessageList
            messages={Array.from({ length: 20 }, (_, index) => ({
              id: String(index),
              user_id: "other",
              display_name: "Alice",
              parent_id: null,
              created_at: "2026-01-02 00:00:00.000",
              content:
                index === previewIndex
                  ? `https://example.com/scroll-${position}`
                  : `Message ${index}\nLine one\nLine two\nLine three`,
            }))}
            isLoading={false}
            isError={false}
            currentUserId="me"
            editingId={null}
            editingText=""
            onEditingTextChange={noop}
            onStartEdit={noop}
            onCancelEdit={noop}
            onSaveEdit={noop}
            onRequestDelete={noop}
            reactions={new Map()}
            kitchenCatalog={null}
            canReact={false}
            onReaction={noop}
            onPickReaction={noop}
            replyCounts={new Map()}
            onOpenThread={noop}
          />
        </div>
      </LocaleProvider>,
    );
    const scroller = screen.getByTestId("message-scroll");
    const distanceFromBottom = () =>
      scroller.scrollHeight - scroller.clientHeight - scroller.scrollTop;
    await waitFor(() => expect(distanceFromBottom()).toBeLessThanOrEqual(1));
    await nextFrame();
    if (position === "returned") {
      scroller.scrollTop = 0;
      fireEvent.scroll(scroller);
      await nextFrame();
      scroller.scrollTop = scroller.scrollHeight;
      fireEvent.scroll(scroller);
      await nextFrame();
    } else if (!followsBottom) {
      scroller.scrollTop = 0;
      fireEvent.scroll(scroller);
      await nextFrame();
      const row = scroller.querySelector<HTMLElement>('[data-message-id="1"]')!;
      const bounds = row.getBoundingClientRect();
      scroller.scrollTop =
        scroller.scrollTop +
        (position === "above" ? bounds.bottom + 10 : bounds.top + 10) -
        scroller.getBoundingClientRect().top;
      fireEvent.scroll(scroller);
      await nextFrame();
    }
    const anchor = scroller.querySelector<HTMLElement>(
      `[data-message-id="${position === "above" ? 2 : previewIndex}"]`,
    )!;
    const before = anchor.getBoundingClientRect().top;
    const expectPosition = async () => {
      await nextFrame();
      await waitFor(() => {
        if (followsBottom) expect(distanceFromBottom()).toBeLessThanOrEqual(1);
        else expect(Math.abs(anchor.getBoundingClientRect().top - before)).toBeLessThanOrEqual(1);
      });
    };
    await act(async () => {
      resolveMetadata({
        title: "Delayed URL card",
        description: "The card expands after metadata and a landscape image load.",
        image: "https://example.com/scroll.png",
      });
    });
    const card = await screen.findByRole("link", { name: /Delayed URL card/ });
    await expectPosition();
    resolveImage();
    await waitFor(() => expect(card).toHaveClass("flex-col"));
    await expectPosition();
    expect(screen.queryByRole("button", { name: "新着メッセージ" })).toBeNull();
  },
);
