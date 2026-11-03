import "../../styles.css";
import { render, screen, waitFor } from "@testing-library/react";
import { expect, it, vi } from "vite-plus/test";
import { http, HttpResponse } from "msw";
import { mockServer, startMockServer } from "../../test/browser-setup";
import { ActivityPubEmoji } from "./ActivityPubEmoji";

vi.mock("../../lib/electric/collections-context", () => ({
  useCollections: () => ({ config: { apiBaseUrl: "http://localhost:8000" } }),
  useOptionalCollections: () => null,
}));

it.each([16, "1.5em"])(
  "keeps the wide emoji slot when the image fails at size %s",
  async (size) => {
    await startMockServer();
    let release!: () => void;
    const pending = new Promise<void>((resolve) => {
      release = resolve;
    });
    mockServer.use(
      http.get("*/api/activitypub/emoji-images/:id", async () => {
        await pending;
        return new HttpResponse(null, { status: 404 });
      }),
    );
    const imageId = "a".repeat(63) + (typeof size === "number" ? "1" : "2");
    render(
      <div style={{ fontSize: 14 }}>
        <ActivityPubEmoji
          emoji={{ image_id: imageId, name: ":wide:", url: "", width: 120, height: 40 }}
          size={size}
        />
        <span data-testid="following">After</span>
      </div>,
    );
    await document.fonts.ready;
    const initial = screen
      .getByRole("img", { name: ":wide:" })
      .parentElement!.getBoundingClientRect();
    const following = screen.getByTestId("following").getBoundingClientRect();
    expect(initial.width / initial.height).toBeCloseTo(3);
    release();
    await waitFor(() =>
      expect(screen.queryByRole("img", { name: ":wide:" })).not.toBeInTheDocument(),
    );
    const fallback = screen.getByText(":wide:").getBoundingClientRect();
    expect(fallback.width).toBe(initial.width);
    expect(fallback.height).toBe(initial.height);
    expect(screen.getByTestId("following").getBoundingClientRect().left).toBe(following.left);
    expect(screen.getByTestId("following").getBoundingClientRect().top).toBe(following.top);
  },
);
