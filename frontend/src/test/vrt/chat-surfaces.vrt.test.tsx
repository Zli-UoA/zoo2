import { render } from "@testing-library/react";
import { describe, expect, it } from "vite-plus/test";
import { page } from "vite-plus/test/browser/context";
import type { ComponentProps, ReactNode } from "react";

import "../../styles.css";
import { LocaleProvider } from "../../components/i18n/LocaleProvider";
import { Composer } from "../../components/chat/messages/Composer";
import { MessageRow } from "../../components/chat/messages/MessageRow";
import type { MessageItem } from "../../components/chat/messages/messageGrouping";

const messageItem: MessageItem = {
  message: {
    id: "message-1",
    user_id: "user-1",
    content:
      "おはようございます。今日のリリースは 18:00 です。詳細は https://example.com/docs を参照してください。",
    created_at: "2026-01-02 09:00:00.000",
    parent_id: null,
    user_name: "alice",
    display_name: "Alice",
  },
  isGroupStart: true,
  isGroupEnd: true,
  showDateSeparator: false,
};

const reactionItem: MessageItem = {
  message: {
    id: "message-2",
    user_id: "user-2",
    content: "デプロイが完了しました 🎉",
    created_at: "2026-01-02 09:05:00.000",
    parent_id: null,
    user_name: "bob",
    display_name: "Bob",
  },
  isGroupStart: true,
  isGroupEnd: true,
  showDateSeparator: false,
};

function noop() {}

function Surface({ theme, children }: { theme: "light" | "dark"; children: ReactNode }) {
  return (
    <div
      data-testid="vrt-root"
      className={`${theme === "dark" ? "dark " : ""}min-h-screen bg-background p-4 text-foreground md:p-8`}
    >
      <div className="mx-auto max-w-3xl">
        <LocaleProvider>{children}</LocaleProvider>
      </div>
    </div>
  );
}

async function renderSurface(
  state: { theme: "light" | "dark"; width: number; height: number },
  children: ReactNode,
) {
  await page.viewport(state.width, state.height);
  render(<Surface theme={state.theme}>{children}</Surface>);
  // Layout requests the glyph subsets before we wait for the webfonts.
  await new Promise<void>((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
  });
  await document.fonts.ready;
}

function messageRow(overrides: Partial<ComponentProps<typeof MessageRow>> = {}) {
  return (
    <MessageRow
      item={messageItem}
      isOwn={false}
      isEditing={false}
      editingText=""
      onEditingTextChange={noop}
      onStartEdit={noop}
      onCancelEdit={noop}
      onSaveEdit={noop}
      onRequestDelete={noop}
      reactions={[]}
      kitchenCatalog={null}
      canReact
      onReaction={noop}
      onPickReaction={noop}
      {...overrides}
    />
  );
}

describe("chat surfaces", () => {
  it("message row desktop light", async () => {
    await renderSurface({ theme: "light", width: 1440, height: 900 }, messageRow());

    await expect(page).toMatchScreenshot("message-row-desktop-light");
  });

  it("message row reactions desktop light", async () => {
    await renderSurface(
      { theme: "light", width: 1440, height: 900 },
      messageRow({
        item: reactionItem,
        replyCount: 3,
        onOpenThread: noop,
        reactions: [
          { emoji: "👍", count: 3, ownId: "reaction-1" },
          { emoji: "🎉", count: 1, ownId: null },
        ],
      }),
    );

    await expect(page).toMatchScreenshot("message-row-reactions-desktop-light");
  });

  it("message row reactions desktop dark", async () => {
    await renderSurface(
      { theme: "dark", width: 1440, height: 900 },
      messageRow({
        item: reactionItem,
        replyCount: 3,
        onOpenThread: noop,
        reactions: [
          { emoji: "👍", count: 3, ownId: "reaction-1" },
          { emoji: "🎉", count: 1, ownId: null },
        ],
      }),
    );

    await expect(page).toMatchScreenshot("message-row-reactions-desktop-dark");
  });

  it("message row reactions mobile light", async () => {
    await renderSurface(
      { theme: "light", width: 390, height: 844 },
      messageRow({
        item: reactionItem,
        replyCount: 3,
        onOpenThread: noop,
        reactions: [
          { emoji: "👍", count: 3, ownId: "reaction-1" },
          { emoji: "🎉", count: 1, ownId: null },
        ],
      }),
    );

    await expect(page).toMatchScreenshot("message-row-reactions-mobile-light");
  });

  it("composer mobile dark", async () => {
    await renderSurface(
      { theme: "dark", width: 390, height: 844 },
      <Composer onSend={async () => true} />,
    );

    await expect(page).toMatchScreenshot("composer-mobile-dark");
  });
});
