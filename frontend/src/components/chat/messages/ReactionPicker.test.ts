// @vitest-environment happy-dom
import { act, createElement, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vite-plus/test";

import { LocaleProvider } from "../../i18n/LocaleProvider";
import { RECENT_EMOJIS_STORAGE_KEY } from "../../../lib/emoji/recents";
import { ReactionPicker } from "./ReactionPicker";

vi.mock("../../../lib/emoji/workspace", () => ({
  useWorkspaceEmojis: () => ({
    emojis: [
      {
        name: "zoo_wave",
        url: "/api/emoji-images/fixture",
        category: "Greetings",
        aliases: ["挨拶"],
      },
    ],
    loading: false,
    failed: false,
    retry: () => {},
    imageUrl: (url: string) => url,
  }),
}));

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
    defaultEmojiDataResolver: vi.fn(),
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

async function flush(action: () => unknown) {
  await act(async () => {
    action();
    await Promise.resolve();
  });
}

async function click(element: Element) {
  await flush(() =>
    element.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true })),
  );
}

function buttonWithText(text: string): HTMLButtonElement {
  const button = Array.from(document.querySelectorAll("button")).find(
    (candidate) => candidate.textContent === text,
  );
  if (!(button instanceof HTMLButtonElement)) throw new Error(`button not found: ${text}`);
  return button;
}

let root: Root;
let container: HTMLDivElement;
let selected: string[];
let openChanges: boolean[];

beforeEach(async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  localStorage.clear();
  localStorage.setItem("zoo-locale", "ja");
  selected = [];
  openChanges = [];
  const onSelect = (emoji: string) => {
    selected.push(emoji);
  };
  const onOpenChange = (open: boolean) => {
    openChanges.push(open);
  };
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  await flush(() =>
    root.render(
      createElement(LocaleProvider, {
        children: createElement(ReactionPicker, {
          open: true,
          onOpenChange,
          onSelect,
        }),
      }),
    ),
  );
});

afterEach(async () => {
  await flush(() => root.unmount());
  container.remove();
  localStorage.clear();
  vi.unstubAllGlobals();
});

describe("ReactionPicker", () => {
  it("searches custom emoji by alias and selects the canonical name", async () => {
    await click(buttonWithText("カスタム絵文字"));
    const input = document.querySelector<HTMLInputElement>('input[aria-label="絵文字を検索"]')!;
    await flush(() => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(
        input,
        "挨拶",
      );
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    expect(document.body.textContent).toContain("Greetings");
    await click(document.querySelector('button[aria-label=":zoo_wave:"]')!);
    expect(selected).toEqual([":zoo_wave:"]);
    expect(openChanges).toEqual([false]);
  });
  it("shows all tabs and selects an emoji while recording it as recent", async () => {
    const tabs = Array.from(document.querySelectorAll('[role="tab"]'));
    expect(tabs.map((tab) => tab.textContent)).toEqual([
      "絵文字",
      "Emoji Kitchen",
      "カスタム絵文字",
    ]);

    await click(document.querySelector('[data-testid="mock-emoji"]')!);

    expect(selected).toEqual(["😀"]);
    expect(openChanges).toEqual([false]);
    expect(JSON.parse(localStorage.getItem(RECENT_EMOJIS_STORAGE_KEY)!)).toEqual(["😀"]);
    expect(document.querySelector('[aria-label="最近使った絵文字"]')?.textContent).toContain("😀");
  });

  it("keeps Emoji Kitchen working and selects a combination", async () => {
    await click(buttonWithText("Emoji Kitchen"));
    expect(document.querySelector('[data-testid="mock-emoji"]')).toBeNull();
    expect(document.body.textContent).toContain("1つ目の絵文字");

    const image = document.querySelector("img")!;
    await flush(() => image.dispatchEvent(new Event("load")));

    const add = buttonWithText("この組み合わせを追加");
    expect(add.disabled).toBe(false);
    await click(add);

    expect(selected).toEqual(["kitchen:1f431_1f600"]);
    expect(openChanges).toEqual([false]);
  });

  it("switches back to the emoji tab", async () => {
    await click(buttonWithText("Emoji Kitchen"));
    await click(buttonWithText("絵文字"));
    expect(document.querySelector('[data-testid="mock-emoji"]')).not.toBeNull();
    expect(document.body.textContent).not.toContain("1つ目の絵文字");
  });
});
