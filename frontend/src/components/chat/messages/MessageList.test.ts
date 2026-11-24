// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { expect, it, vi } from "vite-plus/test";

import { LocaleProvider, useLocale } from "../../i18n/LocaleProvider";
import { MentionUserNamesContext } from "../../../lib/e2ee/messageDecryption";
import { MessageList } from "./MessageList";
import type { FeedMessage } from "./messageFormat";

it.each([
  { currentUserId: "viewer", viewerId: undefined, highlighted: true },
  { currentUserId: null, viewerId: "viewer", highlighted: true },
  { currentUserId: "viewer", viewerId: null, highlighted: false },
])(
  "passes mention identity through the list: $currentUserId / $viewerId",
  ({ currentUserId, viewerId, highlighted }) => {
    const noop = () => {};
    const html = renderToStaticMarkup(
      createElement(MentionUserNamesContext.Provider, {
        value: new Map([["viewer", "alice"]]),
        children: createElement(LocaleProvider, {
          children: createElement(MessageList, {
            messages: [
              {
                id: "mention-1",
                user_id: "other",
                content: "@alice hello",
                created_at: "2026-01-02 00:00:00.000",
                parent_id: null,
                display_name: "Bob",
              },
            ],
            isLoading: false,
            isError: false,
            currentUserId,
            viewerId,
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
          }),
        }),
      }),
    );
    expect(html).toContain("@alice");
    expect(html.includes("bg-mention/[0.08]")).toBe(highlighted);
  },
);

it.each([
  { currentUserId: "viewer", viewerId: undefined, mine: true, ownActions: true },
  { currentUserId: null, viewerId: "viewer", mine: true, ownActions: false },
  { currentUserId: "viewer", viewerId: null, mine: true, ownActions: true },
  { currentUserId: null, viewerId: null, mine: false, ownActions: false },
])(
  "identifies viewer reactions without enabling author actions: $currentUserId / $viewerId",
  ({ currentUserId, viewerId, mine, ownActions }) => {
    localStorage.setItem("zoo-locale", "ja");
    const noop = () => {};
    try {
      const html = renderToStaticMarkup(
        createElement(LocaleProvider, {
          children: createElement(MessageList, {
            messages: [
              {
                id: "react-1",
                user_id: "viewer",
                content: "hello",
                created_at: "2026-01-02 00:00:00.000",
                parent_id: null,
                display_name: "Alice",
                display_data: {
                  emojis: {},
                  attachments: [],
                  reply_count: 0,
                  reactions: [
                    {
                      id: "r1",
                      message_id: "react-1",
                      user_id: "viewer",
                      emoji: "👍",
                      created_at: "2026-01-02 00:00:00.000",
                      user_name: "alice",
                      display_name: "Alice",
                    },
                  ],
                },
              },
            ],
            isLoading: false,
            isError: false,
            currentUserId,
            viewerId,
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
          }),
        }),
      );
      expect(html.includes('aria-pressed="true"')).toBe(mine);
      expect(html.includes("bg-brand-muted")).toBe(mine);
      expect(html.includes("自分もリアクション済み")).toBe(mine);
      expect(html.includes('aria-label="編集"')).toBe(ownActions);
    } finally {
      localStorage.clear();
    }
  },
);

it.each([
  { label: "active row", threadArchiveAt: null, archived: false },
  { label: "archived thread root", threadArchiveAt: "2026-01-03T00:00:00Z", archived: true },
])("disables row reactions on $label", ({ threadArchiveAt, archived }) => {
  localStorage.setItem("zoo-locale", "ja");
  const noop = () => {};
  try {
    const html = renderToStaticMarkup(
      createElement(LocaleProvider, {
        children: createElement(MessageList, {
          messages: [
            {
              id: "react-archived",
              user_id: "other",
              content: "hello",
              created_at: "2026-01-02 00:00:00.000",
              parent_id: null,
              thread_archive_at: threadArchiveAt,
              display_name: "Bob",
              display_data: {
                emojis: {},
                attachments: [],
                reply_count: 0,
                reactions: [
                  {
                    id: "r1",
                    message_id: "react-archived",
                    user_id: "other",
                    emoji: "👍",
                    created_at: "2026-01-02 00:00:00.000",
                    user_name: "bob",
                    display_name: "Bob",
                  },
                ],
              },
            },
          ],
          isLoading: false,
          isError: false,
          currentUserId: null,
          viewerId: "viewer",
          editingId: null,
          editingText: "",
          onEditingTextChange: noop,
          onStartEdit: noop,
          onCancelEdit: noop,
          onSaveEdit: noop,
          onRequestDelete: noop,
          reactions: new Map(),
          kitchenCatalog: null,
          canReact: true,
          onReaction: noop,
          onPickReaction: noop,
        }),
      }),
    );
    expect(html.includes('aria-disabled="true"')).toBe(archived);
    expect(html.includes('aria-disabled="false"')).toBe(!archived);
    expect(html.includes('disabled=""')).toBe(archived);
  } finally {
    localStorage.clear();
  }
});

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

it("shows only the empty title when composing is unavailable", () => {
  localStorage.setItem("zoo-locale", "ja");
  const noop = () => {};
  const render = (canCompose?: boolean) =>
    renderToStaticMarkup(
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
          onOpenThread: noop,
          canCompose,
        }),
      }),
    );

  try {
    const readOnly = render(false);
    expect(readOnly).toContain("まだメッセージがありません");
    expect(readOnly).not.toContain("最初のメッセージを送ってみましょう。");

    expect(render()).toContain("最初のメッセージを送ってみましょう。");
  } finally {
    localStorage.clear();
  }
});
