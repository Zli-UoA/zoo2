import "../../../styles.css";
import { delay, http, HttpResponse } from "msw";
import { mockServer, startMockServer } from "../../../test/browser-setup";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ComponentProps, ReactNode } from "react";
import { describe, expect, it, vi } from "vite-plus/test";

import { LocaleProvider } from "../../i18n/LocaleProvider";
import { RECENT_EMOJIS_STORAGE_KEY } from "../../../lib/emoji/recents";
import { ReactionPicker } from "./ReactionPicker";

vi.mock("frimousse", async () => {
  const { createContext, createElement: element, useContext } = await import("react");
  type Select = ((emoji: { emoji: string; label: string }) => void) | undefined;
  const SelectContext = createContext<Select>(undefined);
  function Root({
    children,
    onEmojiSelect,
  }: {
    children?: ReactNode;
    onEmojiSelect?: (emoji: { emoji: string; label: string }) => void;
  }) {
    return element(
      SelectContext.Provider,
      { value: onEmojiSelect },
      element("div", null, children),
    );
  }
  // The grid only exists on the emoji panels, like the real frimousse viewport.
  function Viewport({ children }: { children?: ReactNode }) {
    const onEmojiSelect = useContext(SelectContext);
    return element(
      "div",
      null,
      element(
        "button",
        {
          type: "button",
          "data-testid": "mock-emoji",
          onClick: () => onEmojiSelect?.({ emoji: "😀", label: "grinning face" }),
        },
        "😀",
      ),
      children,
    );
  }
  return {
    defaultEmojiDataResolver: vi.fn(),
    EmojiPicker: {
      Root,
      Search: ({ value, onChange, placeholder }: ComponentProps<"input">) =>
        element("input", {
          type: "search",
          value,
          onChange,
          placeholder,
          "aria-label": placeholder,
        }),
      Viewport,
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

async function createSolidPng(width: number, height: number): Promise<ArrayBuffer> {
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("2d canvas context is unavailable");
  ctx.fillStyle = "#ff0000";
  ctx.fillRect(0, 0, width, height);
  const blob = await new Promise<Blob>((resolve, reject) => {
    canvas.toBlob(
      (result) => (result ? resolve(result) : reject(new Error("canvas.toBlob returned null"))),
      "image/png",
    );
  });
  return blob.arrayBuffer();
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
    await userEvent.type(screen.getByRole("searchbox", { name: "絵文字を検索" }), "挨拶");
    expect(screen.queryByRole("button", { name: ":other:" })).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: ":party-parrot:" }));
    expect(onSelect).toHaveBeenCalledWith(":party-parrot:");
    expect(onOpenChange).toHaveBeenCalledWith(false);
    expect(JSON.parse(localStorage.getItem(RECENT_EMOJIS_STORAGE_KEY)!)).toEqual([
      ":party-parrot:",
    ]);
  });
  it("searches custom emoji from the emoji tab with the shared search box", async () => {
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
        ]),
      ),
    );
    const { onSelect } = renderPicker();
    await userEvent.type(screen.getByRole("searchbox", { name: "絵文字を検索" }), "挨拶");
    await userEvent.click(await screen.findByRole("button", { name: ":party-parrot:" }));
    expect(onSelect).toHaveBeenCalledWith(":party-parrot:");
  });
  it("shows the search box above the tabs and no description on the emoji tab", () => {
    renderPicker();
    const search = screen.getByRole("searchbox", { name: "絵文字を検索" });
    const tabs = screen.getByRole("tablist");
    expect(search.compareDocumentPosition(tabs) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(screen.queryByText("すべての絵文字から選べます。")).toBeNull();
  });
  it("shows all tabs and selects an emoji while recording it as recent", async () => {
    const { onSelect, onOpenChange } = renderPicker();

    expect(screen.getAllByRole("tab").map((tab) => tab.textContent)).toEqual([
      "絵文字",
      "カスタム絵文字",
      "Emoji Kitchen",
    ]);

    await userEvent.click(screen.getByTestId("mock-emoji"));

    expect(onSelect).toHaveBeenCalledWith("😀");
    expect(onOpenChange).toHaveBeenCalledWith(false);
    expect(JSON.parse(localStorage.getItem(RECENT_EMOJIS_STORAGE_KEY)!)).toEqual(["😀"]);
    // ピッカー側には「最近」を出さない（保存先のシートで使う）。
    expect(screen.queryByLabelText("最近使った絵文字")).toBeNull();
  });

  it("keeps Emoji Kitchen working and selects a combination", async () => {
    const { onSelect, onOpenChange } = renderPicker();

    await userEvent.click(screen.getByRole("tab", { name: "Emoji Kitchen" }));
    await waitFor(() => expect(screen.queryByRole("button", { name: "😀" })).toBeNull());
    expect(await screen.findByText("1つ目の絵文字")).toBeVisible();

    fireEvent.load(screen.getByRole("img", { name: "組み合わせのプレビュー" }));

    const add = screen.getByRole("button", { name: "この組み合わせを追加" });
    expect(add.hasAttribute("disabled")).toBe(false);
    await userEvent.click(add);

    expect(onSelect).toHaveBeenCalledWith("kitchen:1f431_1f600");
    expect(onOpenChange).toHaveBeenCalledWith(false);
    expect(JSON.parse(localStorage.getItem(RECENT_EMOJIS_STORAGE_KEY)!)).toEqual([
      "kitchen:1f431_1f600",
    ]);
  });

  it("switches back to the emoji tab", async () => {
    renderPicker();

    await userEvent.click(screen.getByRole("tab", { name: "Emoji Kitchen" }));
    await userEvent.click(screen.getByRole("tab", { name: "絵文字" }));

    expect(screen.getByTestId("mock-emoji")).not.toBeNull();
    await waitFor(() => expect(screen.queryByText("1つ目の絵文字")).toBeNull());
  });

  it("reserves custom emoji image dimensions from metadata before the image loads", async () => {
    await startMockServer();
    mockServer.use(
      http.get("*/api/emojis", () =>
        HttpResponse.json([
          {
            name: "wide-emoji",
            url: "https://www.gstatic.com/pending-wide.png",
            category: "Wide",
            aliases: [],
            width: 3,
            height: 1,
          },
          {
            name: "ultra-wide-emoji",
            url: "https://www.gstatic.com/pending-ultra.png",
            category: "Wide",
            aliases: [],
            width: 1000,
            height: 10,
          },
          {
            name: "legacy-emoji",
            url: "https://www.gstatic.com/pending-legacy.png",
            category: "Legacy",
            aliases: [],
            width: null,
            height: null,
          },
        ]),
      ),
      http.get("https://www.gstatic.com/pending-*.png", async () => {
        await delay("infinite");
        return new HttpResponse(null);
      }),
    );

    renderPicker();
    await userEvent.click(screen.getByRole("tab", { name: "カスタム絵文字" }));

    const wide = (await screen.findByRole("img", { name: ":wide-emoji:" })) as HTMLImageElement;
    expect(wide.complete).toBe(false);
    expect(wide.style.getPropertyValue("--emoji-aspect-ratio")).toBe("auto 3 / 1");
    expect(Math.abs(wide.getBoundingClientRect().width - 84)).toBeLessThanOrEqual(1);
    expect(Math.abs(wide.getBoundingClientRect().height - 28)).toBeLessThanOrEqual(1);

    const ultra = screen.getByRole("img", { name: ":ultra-wide-emoji:" }) as HTMLImageElement;
    expect(Math.abs(ultra.getBoundingClientRect().width - 160)).toBeLessThanOrEqual(1);

    const legacy = screen.getByRole("img", { name: ":legacy-emoji:" }) as HTMLImageElement;
    expect(legacy.style.getPropertyValue("--emoji-aspect-ratio")).toBe("");
  });

  it("uses the decoded image aspect ratio after load when stored metadata orientation differs", async () => {
    const [landscapeBytes, portraitBytes] = await Promise.all([
      createSolidPng(300, 100),
      createSolidPng(100, 300),
    ]);
    await startMockServer();
    mockServer.use(
      http.get("*/api/emojis", () =>
        HttpResponse.json([
          {
            name: "rotated-landscape",
            url: "https://www.gstatic.com/rotated-landscape.png",
            category: "Rotated",
            aliases: [],
            width: 100,
            height: 300,
          },
          {
            name: "rotated-portrait",
            url: "https://www.gstatic.com/rotated-portrait.png",
            category: "Rotated",
            aliases: [],
            width: 300,
            height: 100,
          },
        ]),
      ),
      http.get("https://www.gstatic.com/rotated-landscape.png", () =>
        HttpResponse.arrayBuffer(landscapeBytes, { headers: { "Content-Type": "image/png" } }),
      ),
      http.get("https://www.gstatic.com/rotated-portrait.png", () =>
        HttpResponse.arrayBuffer(portraitBytes, { headers: { "Content-Type": "image/png" } }),
      ),
    );

    renderPicker();
    await userEvent.click(screen.getByRole("tab", { name: "カスタム絵文字" }));

    const landscape = (await screen.findByRole("img", {
      name: ":rotated-landscape:",
    })) as HTMLImageElement;
    const portrait = screen.getByRole("img", { name: ":rotated-portrait:" }) as HTMLImageElement;

    await waitFor(() => {
      expect(landscape.complete).toBe(true);
      expect(landscape.naturalWidth).toBe(300);
      expect(landscape.naturalHeight).toBe(100);
      expect(portrait.complete).toBe(true);
      expect(portrait.naturalWidth).toBe(100);
      expect(portrait.naturalHeight).toBe(300);
    });

    expect(Math.abs(landscape.getBoundingClientRect().width - 84)).toBeLessThanOrEqual(1);
    expect(Math.abs(landscape.getBoundingClientRect().height - 28)).toBeLessThanOrEqual(1);
    expect(Math.abs(portrait.getBoundingClientRect().width - 28 / 3)).toBeLessThanOrEqual(1.5);
    expect(Math.abs(portrait.getBoundingClientRect().height - 28)).toBeLessThanOrEqual(1);
  });
});
