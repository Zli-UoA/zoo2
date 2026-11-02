import { http, HttpResponse } from "msw";
import { mockServer, startMockServer } from "../../../test/browser-setup";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";
import { describe, expect, it, vi } from "vite-plus/test";

import { LocaleProvider } from "../../i18n/LocaleProvider";
import { RECENT_EMOJIS_STORAGE_KEY } from "../../../lib/emoji/recents";
import { ReactionPicker } from "./ReactionPicker";

vi.mock("frimousse", async () => {
  const { createElement: element } = await import("react");
  function Root({
    children,
    onEmojiSelect,
  }: {
    children?: ReactNode;
    onEmojiSelect?: (emoji: { emoji: string; label: string }) => void;
  }) {
    return element(
      "div",
      null,
      children,
      element(
        "button",
        {
          type: "button",
          "data-testid": "mock-emoji",
          onClick: () => onEmojiSelect?.({ emoji: "😀", label: "grinning face" }),
        },
        "😀",
      ),
    );
  }
  function Passthrough({ children }: { children?: ReactNode }) {
    return element("div", null, children);
  }
  return {
    EmojiPicker: {
      Root,
      Search: () => null,
      Viewport: Passthrough,
      Loading: () => null,
      Empty: () => null,
      List: () => null,
      SkinToneSelector: () => null,
    },
  };
});

vi.mock("../../../hooks/useKitchenCatalog", () => ({
  useKitchenCatalog: () => ({
    catalog: {
      emojis: [
        ["1f600", "😀", "grinning face"],
        ["1f431", "🐱", "cat face"],
      ],
      dates: ["20230301"],
      pairs: { "0_1": 0 },
    },
    error: false,
    retry: () => {},
  }),
}));

function renderPicker() {
  const onSelect = vi.fn();
  const onOpenChange = vi.fn();
  render(
    <LocaleProvider>
      <ReactionPicker open onOpenChange={onOpenChange} onSelect={onSelect} />
    </LocaleProvider>,
  );
  return { onSelect, onOpenChange };
}

describe("ReactionPicker", () => {
  it("searches registered custom emoji by alias and selects its canonical name", async () => {
    await startMockServer();
    mockServer.use(
      http.get("*/api/emojis", () =>
        HttpResponse.json([
          {
            name: "party-parrot",
            url: "https://www.gstatic.com/party-parrot.png",
            category: "Greetings",
            aliases: ["挨拶"],
          },
          {
            name: "other",
            url: "https://www.gstatic.com/other.png",
            category: "Other",
            aliases: [],
          },
        ]),
      ),
    );
    const { onSelect, onOpenChange } = renderPicker();
    await userEvent.click(screen.getByRole("tab", { name: "カスタム絵文字" }));
    await screen.findByRole("button", { name: ":party-parrot:" });
    await userEvent.type(screen.getByRole("textbox", { name: "絵文字を検索" }), "挨拶");
    expect(screen.queryByRole("button", { name: ":other:" })).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: ":party-parrot:" }));
    expect(onSelect).toHaveBeenCalledWith(":party-parrot:");
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });
  it("shows all tabs and selects an emoji while recording it as recent", async () => {
    const { onSelect, onOpenChange } = renderPicker();

    expect(screen.getAllByRole("tab").map((tab) => tab.textContent)).toEqual([
      "絵文字",
      "Emoji Kitchen",
      "カスタム絵文字",
    ]);

    await userEvent.click(screen.getByTestId("mock-emoji"));

    expect(onSelect).toHaveBeenCalledWith("😀");
    expect(onOpenChange).toHaveBeenCalledWith(false);
    expect(JSON.parse(localStorage.getItem(RECENT_EMOJIS_STORAGE_KEY)!)).toEqual(["😀"]);
    expect(screen.getByLabelText("最近使った絵文字").textContent).toContain("😀");
  });

  it("keeps Emoji Kitchen working and selects a combination", async () => {
    const { onSelect, onOpenChange } = renderPicker();

    await userEvent.click(screen.getByRole("tab", { name: "Emoji Kitchen" }));
    expect(screen.queryByTestId("mock-emoji")).toBeNull();
    expect(document.body.textContent).toContain("1つ目の絵文字");

    fireEvent.load(screen.getByRole("img", { name: "組み合わせのプレビュー" }));

    const add = screen.getByRole("button", { name: "この組み合わせを追加" });
    expect(add.hasAttribute("disabled")).toBe(false);
    await userEvent.click(add);

    expect(onSelect).toHaveBeenCalledWith("kitchen:1f431_1f600");
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  it("switches back to the emoji tab", async () => {
    renderPicker();

    await userEvent.click(screen.getByRole("tab", { name: "Emoji Kitchen" }));
    await userEvent.click(screen.getByRole("tab", { name: "絵文字" }));

    expect(screen.getByTestId("mock-emoji")).not.toBeNull();
    await waitFor(() => expect(screen.queryByText("1つ目の絵文字")).toBeNull());
  });
});
