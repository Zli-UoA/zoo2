import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vite-plus/test";
import { LocaleProvider } from "../i18n/LocaleProvider";
import { AppBlocks } from "./AppBlocks";
import { ReactionEmoji } from "./messages/ReactionEmoji";

const { api, collections } = vi.hoisted(() => ({
  api: { request: vi.fn(), get: vi.fn() },
  collections: { config: { apiBaseUrl: "" } },
}));
vi.mock("../../lib/electric/collections-context", () => ({
  useCollections: () => collections,
  useOptionalCollections: () => collections,
}));
vi.mock("../../lib/api/client", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../lib/api/client")>()),
  createConfiguredApiClient: () => api,
}));

describe("App interaction regressions", () => {
  beforeEach(() => vi.clearAllMocks());
  it("keeps modal input after a button action and closes only after submission", async () => {
    api.request.mockResolvedValue({ event_id: "event" });
    const done = vi.fn();
    render(
      <LocaleProvider>
        <AppBlocks
          appId="app"
          surfaceId="modal"
          channelId="channel"
          submit
          onDone={done}
          blocks={[
            { type: "input", action_id: "draft", label: "Draft" },
            { type: "button", action_id: "preview", label: "Preview" },
          ]}
        />
      </LocaleProvider>,
    );
    await userEvent.type(screen.getByRole("textbox", { name: "Draft" }), "keep my input");
    await userEvent.click(screen.getByRole("button", { name: "Preview" }));
    await waitFor(() => expect(api.request).toHaveBeenCalledTimes(1));
    expect(done).not.toHaveBeenCalled();
    expect((screen.getByRole("textbox", { name: "Draft" }) as HTMLInputElement).value).toBe(
      "keep my input",
    );
    await userEvent.click(screen.getByRole("button", { name: "送信" }));
    await waitFor(() => expect(done).toHaveBeenCalledTimes(1));
    expect(api.request.mock.calls[1]?.[1].body).toMatchObject({
      kind: "submit",
      values: { draft: "keep my input" },
    });
  });
  it("loads hyphenated custom emoji names as images", async () => {
    api.get.mockResolvedValue([
      {
        name: "party-parrot",
        url: "https://www.gstatic.com/review-emoji.png",
        category: "",
        aliases: [],
      },
    ]);
    render(<ReactionEmoji emoji=":party-parrot:" catalog={null} />);
    expect((await screen.findByRole("img", { name: ":party-parrot:" })).getAttribute("src")).toBe(
      "https://www.gstatic.com/review-emoji.png",
    );
    expect(api.get).toHaveBeenCalledWith("/api/emojis");
  });

  it("refreshes an already rendered custom emoji after a registry edit", async () => {
    api.get.mockResolvedValue([
      {
        name: "party-parrot",
        url: "https://www.gstatic.com/before.png",
        category: "",
        aliases: [],
      },
    ]);
    render(<ReactionEmoji emoji=":party-parrot:" catalog={null} />);
    const image = await screen.findByRole("img", { name: ":party-parrot:" });
    expect(image.getAttribute("src")).toBe("https://www.gstatic.com/before.png");
    api.get.mockResolvedValue([
      { name: "party-parrot", url: "https://www.gstatic.com/after.png", category: "", aliases: [] },
    ]);
    window.dispatchEvent(new Event("workspace-emojis-changed"));
    await waitFor(() =>
      expect(image.getAttribute("src")).toBe("https://www.gstatic.com/after.png"),
    );
  });
});
