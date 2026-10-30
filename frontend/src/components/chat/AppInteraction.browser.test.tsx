import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vite-plus/test";
import { LocaleProvider } from "../i18n/LocaleProvider";
import { AppBlocks } from "./AppBlocks";
import { ReactionEmoji } from "./messages/ReactionEmoji";

const api = vi.hoisted(() => ({ request: vi.fn(), get: vi.fn() }));
vi.mock("../../lib/electric/collections-context", () => ({
  useCollections: () => ({ config: {} }),
}));
vi.mock("../../lib/api/client", () => ({ createConfiguredApiClient: () => api }));

describe("App interaction regressions", () => {
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
    api.get.mockResolvedValue({ url: "https://www.gstatic.com/review-emoji.png" });
    render(<ReactionEmoji emoji=":party-parrot:" catalog={null} />);
    expect((await screen.findByRole("img", { name: ":party-parrot:" })).getAttribute("src")).toBe(
      "https://www.gstatic.com/review-emoji.png",
    );
    expect(api.get).toHaveBeenCalledWith("/api/emojis/party-parrot", expect.anything());
  });
});
