import { render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vite-plus/test";
import { page } from "vite-plus/test/browser/context";
import { useRef, useState, type ComponentProps, type ReactNode } from "react";

import "../../styles.css";
import { LocaleProvider } from "../../components/i18n/LocaleProvider";
import { ChannelBrowserDialog } from "../../components/chat/ChannelBrowserDialog";
import { ChannelSidebar } from "../../components/chat/ChannelSidebar";
import { MembersPanel } from "../../components/chat/MembersPanel";
import { MessageList } from "../../components/chat/messages/MessageList";
import { Composer } from "../../components/chat/messages/Composer";
import { MessageRow } from "../../components/chat/messages/MessageRow";
import { ReactionPicker } from "../../components/chat/messages/ReactionPicker";
import { ThreadPane } from "../../components/chat/messages/ThreadPane";
import { NotificationCard } from "../../components/chat/notifications/NotificationCard";
import { StoryShell } from "../../../.storybook/shell";
import { StoryRouter } from "../../../.storybook/decorators";
import { CollectionsProvider } from "../../lib/electric/collections-context";
import {
  createStoryCollections,
  storyChannels,
  storyCollections,
  storyNotifications,
  storyUsers,
} from "../../../.storybook/fixtures";
import type { MessageItem } from "../../components/chat/messages/messageGrouping";
import type { FeedMessage } from "../../components/chat/messages/messageFormat";

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

const cardItem: MessageItem = {
  ...messageItem,
  message: {
    ...messageItem.message,
    id: "message-card",
    content: "デプロイが完了しました",
    card: JSON.stringify({
      title: "Deploy complete",
      description: "production へのデプロイが完了しました。",
      source: "Deployment service",
      badge: "success",
    }),
  },
};

const threadRoot: FeedMessage = {
  id: "root-1",
  user_id: "user-carol",
  content: "リリースノートの下書きを共有しました。レビューをお願いします。",
  created_at: "2026-01-02 09:05:00.000",
  parent_id: null,
  user_name: "carol",
  display_name: "Carol",
};

const threadReplies: FeedMessage[] = [
  {
    id: "reply-1",
    user_id: "user-bob",
    content: "確認しました。2 点だけコメントしました。",
    created_at: "2026-01-02 09:06:00.000",
    parent_id: "root-1",
    user_name: "bob",
    display_name: "Bob",
  },
  {
    id: "reply-2",
    user_id: "user-alice",
    content: "ありがとうございます。修正して再共有します。",
    created_at: "2026-01-02 09:07:00.000",
    parent_id: "root-1",
    user_name: "alice",
    display_name: "Alice",
  },
];

function noop() {}

function Surface({
  theme,
  children,
  padded = true,
}: {
  theme: "light" | "dark";
  children: ReactNode;
  padded?: boolean;
}) {
  return (
    <div
      data-testid="vrt-root"
      className={`${theme === "dark" ? "dark " : ""}min-h-screen bg-background text-foreground ${
        padded ? "p-4 md:p-8" : ""
      }`}
    >
      <LocaleProvider>{children}</LocaleProvider>
    </div>
  );
}

function useStoryBundle() {
  const [bundle] = useState(() => createStoryCollections(storyCollections));
  return bundle;
}

function WithCollections({
  theme,
  children,
  padded,
}: {
  theme: "light" | "dark";
  children: ReactNode;
  padded?: boolean;
}) {
  const bundle = useStoryBundle();
  return (
    <Surface theme={theme} padded={padded}>
      <CollectionsProvider value={bundle}>
        <StoryRouter>
          <StoryShell>{children}</StoryShell>
        </StoryRouter>
      </CollectionsProvider>
    </Surface>
  );
}

async function renderSurface(
  state: { theme: "light" | "dark"; width: number; height: number },
  children: ReactNode,
) {
  await page.viewport(state.width, state.height);
  render(<Surface theme={state.theme}>{children}</Surface>);
  await document.fonts.ready;
  await new Promise<void>((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
  });
}

async function renderScreen(
  state: { theme: "light" | "dark"; width: number; height: number },
  children: ReactNode,
) {
  await page.viewport(state.width, state.height);
  render(
    <WithCollections theme={state.theme} padded={false}>
      {children}
    </WithCollections>,
  );
  await document.fonts.ready;
  await new Promise<void>((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
  });
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

function threadPane(overrides: Partial<ComponentProps<typeof ThreadPane>> = {}) {
  return (
    <ThreadPane
      root={threadRoot}
      replies={[]}
      isLoading={false}
      isError={false}
      currentUserId="user-alice"
      reactions={new Map()}
      kitchenCatalog={null}
      canReact
      onReaction={noop}
      onPickReaction={noop}
      editingId={null}
      editingText=""
      onEditingTextChange={noop}
      onStartEdit={noop}
      onCancelEdit={noop}
      onSaveEdit={noop}
      onRequestDelete={noop}
      onSendReply={async () => true}
      onClose={noop}
      {...overrides}
    />
  );
}

function messageList(overrides: Partial<ComponentProps<typeof MessageList>> = {}) {
  return (
    <MessageList
      messages={[]}
      isLoading={false}
      isError={false}
      currentUserId={null}
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
      {...overrides}
    />
  );
}

const reactionGroup = {
  key: "notification-reaction",
  latest: storyNotifications.find((row) => row.id === "notification-reaction")!,
  notifications: [storyNotifications.find((row) => row.id === "notification-reaction")!],
  actorIds: ["user-bob"],
};

function NotificationCardVrt() {
  const scrollRoot = useRef<HTMLDivElement>(null);
  return (
    <div ref={scrollRoot} className="mx-auto max-w-3xl">
      <ul>
        <NotificationCard
          group={reactionGroup}
          channel={storyChannels.find((channel) => channel.id === "channel-general")}
          users={new Map(storyUsers.map((user) => [user.id, user]))}
          onRead={noop}
          scrollRoot={scrollRoot}
        />
      </ul>
    </div>
  );
}

describe("catalog surfaces", () => {
  it("message row own desktop light", async () => {
    await renderSurface(
      { theme: "light", width: 1440, height: 900 },
      <div className="mx-auto max-w-3xl">{messageRow({ isOwn: true })}</div>,
    );
    await expect(page).toMatchScreenshot("message-row-own-desktop-light");
  });

  it("message row card desktop light", async () => {
    await renderSurface(
      { theme: "light", width: 1440, height: 900 },
      <div className="mx-auto max-w-3xl">{messageRow({ item: cardItem })}</div>,
    );
    await expect(page).toMatchScreenshot("message-row-card-desktop-light");
  });

  it("message row editing desktop light", async () => {
    await renderSurface(
      { theme: "light", width: 1440, height: 900 },
      <div className="mx-auto max-w-3xl">
        {messageRow({
          isOwn: true,
          isEditing: true,
          editingText: "おはようございます。今日のリリースは 18:00 です。",
        })}
      </div>,
    );
    await expect(page).toMatchScreenshot("message-row-editing-desktop-light");
  });

  it("composer desktop light", async () => {
    await renderSurface(
      { theme: "light", width: 1440, height: 900 },
      <div className="mx-auto max-w-3xl">
        <Composer onSend={async () => true} />
      </div>,
    );
    await expect(page).toMatchScreenshot("composer-desktop-light");
  });

  it("reaction picker desktop light", async () => {
    await renderSurface(
      { theme: "light", width: 1440, height: 900 },
      <ReactionPicker open onOpenChange={noop} onSelect={noop} />,
    );
    await waitFor(() => expect(document.body.textContent).toContain("😀"), { timeout: 5000 });
    await expect(page).toMatchScreenshot("reaction-picker-desktop-light");
  });

  it("thread pane replies desktop light", async () => {
    await renderSurface(
      { theme: "light", width: 1440, height: 900 },
      <div className="flex h-[720px] justify-end bg-muted/30">
        {threadPane({ replies: threadReplies })}
      </div>,
    );
    await expect(page).toMatchScreenshot("thread-pane-replies-desktop-light");
  });

  it("thread pane empty desktop light", async () => {
    await renderSurface(
      { theme: "light", width: 1440, height: 900 },
      <div className="flex h-[720px] justify-end bg-muted/30">{threadPane()}</div>,
    );
    await expect(page).toMatchScreenshot("thread-pane-empty-desktop-light");
  });

  it("notification card mention desktop light", async () => {
    await renderScreen({ theme: "light", width: 1440, height: 900 }, <NotificationCardVrt />);
    await screen.findByText(/リリースノートの下書き/);
    await expect(page).toMatchScreenshot("notification-card-mention-desktop-light");
  });

  it("channel sidebar desktop light", async () => {
    await renderScreen(
      { theme: "light", width: 1440, height: 900 },
      <div className="flex h-screen bg-background">
        <ChannelSidebar channels={storyChannels} loading={false} error={false} />
      </div>,
    );
    await screen.findByText("general");
    await expect(page).toMatchScreenshot("channel-sidebar-desktop-light");
  });

  it("channel sidebar mobile light", async () => {
    await renderScreen(
      { theme: "light", width: 390, height: 844 },
      <div className="flex h-screen bg-background">
        <ChannelSidebar channels={storyChannels} loading={false} error={false} />
      </div>,
    );
    await screen.findByText("general");
    await expect(page).toMatchScreenshot("channel-sidebar-mobile-light");
  });

  it("members panel desktop light", async () => {
    await renderScreen(
      { theme: "light", width: 1440, height: 900 },
      <div className="flex h-screen justify-end bg-background">
        <MembersPanel
          channel={storyChannels.find((channel) => channel.id === "channel-general")!}
          className="flex w-80 border-l"
        />
      </div>,
    );
    await screen.findByText("Alice");
    await expect(page).toMatchScreenshot("members-panel-desktop-light");
  });

  it("channel browser dialog desktop light", async () => {
    await renderScreen(
      { theme: "light", width: 1440, height: 900 },
      <ChannelBrowserDialog
        open
        onOpenChange={noop}
        channels={storyChannels}
        joinedChannelIds={new Set(["channel-general", "channel-design", "channel-secret"])}
      />,
    );
    await screen.findByText("old-announcements");
    await screen.findByText("random");
    await expect(page).toMatchScreenshot("channel-browser-dialog-desktop-light");
  });

  it("message list empty desktop light", async () => {
    await renderSurface(
      { theme: "light", width: 1440, height: 900 },
      <div className="mx-auto flex h-[640px] max-w-3xl flex-col">{messageList()}</div>,
    );
    await expect(page).toMatchScreenshot("message-list-empty-desktop-light");
  });

  it("message list error desktop light", async () => {
    await renderSurface(
      { theme: "light", width: 1440, height: 900 },
      <div className="mx-auto flex h-[640px] max-w-3xl flex-col">
        {messageList({ isError: true })}
      </div>,
    );
    await expect(page).toMatchScreenshot("message-list-error-desktop-light");
  });

  it("message list loading desktop light", async () => {
    await renderSurface(
      { theme: "light", width: 1440, height: 900 },
      <div className="mx-auto flex h-[640px] max-w-3xl flex-col">
        {messageList({ isLoading: true })}
      </div>,
    );
    await expect(page).toMatchScreenshot("message-list-loading-desktop-light");
  });
});
