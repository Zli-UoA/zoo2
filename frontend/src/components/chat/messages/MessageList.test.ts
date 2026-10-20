// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { expect, it, vi } from "vite-plus/test";

import { LocaleProvider, useLocale } from "../../i18n/LocaleProvider";
import { MessageList } from "./MessageList";
import type { FeedMessage } from "./messageFormat";

async function flush(action: () => unknown) {
  await act(async () => {
    action();
    await Promise.resolve();
  });
}

it("announces each new message in its arrival language without repeating it on a language switch", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("requestAnimationFrame", () => 0);
  localStorage.setItem("zoo-locale", "en");
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  let controls: ReturnType<typeof useLocale>;
  function Controls() {
    controls = useLocale();
    return null;
  }
  const message = (id: string): FeedMessage => ({
    id,
    user_id: "other",
    content: id,
    created_at: "2026-01-02 00:00:00.000",
    parent_id: null,
    display_name: "Alice",
  });
  const first = message("one");
  const noop = () => {};
  const render = (messages: FeedMessage[]) =>
    root.render(
      createElement(LocaleProvider, {
        children: [
          createElement(Controls, { key: "controls" }),
          createElement(MessageList, {
            key: "list",
            messages,
            isLoading: false,
            isError: false,
            currentUserId: "me",
            editingId: null,
            editingText: "",
            onEditingTextChange: noop,
            onStartEdit: noop,
            onCancelEdit: noop,
            onSaveEdit: noop,
            onRequestDelete: noop,
            reactions: new Map(),
            kitchenCatalog: null,
            canReact: false,
            onReaction: noop,
            onPickReaction: noop,
            replyCounts: new Map(),
            onOpenThread: noop,
          }),
        ],
      }),
    );
  try {
    await flush(() => render([first]));
    const scroll = container.querySelector<HTMLDivElement>(".overflow-y-auto")!;
    Object.defineProperties(scroll, {
      scrollHeight: { value: 2000 },
      clientHeight: { value: 100 },
    });
    await flush(() => scroll.dispatchEvent(new Event("scroll")));
    await flush(() => render([first, message("two")]));
    const live = container.querySelector('[aria-live="polite"]')!;
    expect(live.textContent).toBe("New message from Alice: “two”");
    await flush(() => controls.setLocale("yue"));
    expect(live.textContent).toBe("New message from Alice: “two”");
    expect(container.textContent).toContain("新訊息");
    await flush(() => render([first, message("two"), message("three")]));
    expect(live.textContent).toBe("Alice 傳咗新訊息：「three」");
  } finally {
    await flush(() => root.unmount());
    container.remove();
    localStorage.clear();
    vi.unstubAllGlobals();
  }
});

it("shows the provided empty description instead of the first-message prompt", () => {
  const noop = () => {};
  const html = renderToStaticMarkup(
    createElement(LocaleProvider, {
      children: createElement(MessageList, {
        messages: [],
        isLoading: false,
        isError: false,
        currentUserId: null,
        editingId: null,
        editingText: "",
        onEditingTextChange: noop,
        onStartEdit: noop,
        onCancelEdit: noop,
        onSaveEdit: noop,
        onRequestDelete: noop,
        reactions: new Map(),
        kitchenCatalog: null,
        canReact: false,
        onReaction: noop,
        onPickReaction: noop,
        replyCounts: new Map(),
        onOpenThread: noop,
        emptyDescription: "アーカイブ済みのため送信できません",
      }),
    }),
  );
  expect(html).toContain("アーカイブ済みのため送信できません");
  expect(html).not.toContain("最初のメッセージを送ってみましょう。");
});
