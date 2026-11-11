import { playForImplementation } from "../../../.storybook/play-for-implementation";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { http, HttpResponse } from "msw";
import { expect, screen, userEvent, within } from "storybook/test";

import { StoryShell } from "../../../.storybook/shell";
import {
  storyChannelUsers,
  storyChannels,
  storyCollections,
  storyMessages,
} from "../../../.storybook/fixtures";
import { ChannelSidebar } from "./ChannelSidebar";

const meta = {
  title: "Features/Channels/Sidebar",
  component: ChannelSidebar,
  tags: ["autodocs"],
  parameters: { a11y: { test: "todo" } },
  argTypes: {
    loading: { control: "boolean" },
    error: { control: "boolean" },
  },
  args: {
    channels: storyChannels,
    loading: false,
    error: false,
  },
  decorators: [
    (Story) => (
      <StoryShell>
        <div className="flex h-[640px] bg-background">
          <Story />
        </div>
      </StoryShell>
    ),
  ],
} satisfies Meta<typeof ChannelSidebar>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};

export const Empty: Story = {
  args: {
    channels: [],
  },
};

export const Loading: Story = {
  args: {
    loading: true,
  },
};

export const LoadError: Story = {
  args: {
    error: true,
  },
};

export const Mobile: Story = {
  globals: { viewport: { value: "mobile" } },
  args: {
    channels: storyChannels,
  },
};

export const UnreadAndArchived: Story = {
  args: {
    channels: storyChannels,
  },
  parameters: {
    collections: {
      ...storyCollections,
      channelUsers: [
        ...storyChannelUsers,
        {
          user_id: "user-alice",
          channel_id: "channel-archive",
          joined_at: "2026-01-01 00:00:00.000",
        },
      ],
    },
    msw: {
      handlers: [
        http.get("*/api/channel-read-states", () =>
          HttpResponse.json([
            { channel_id: "channel-general", last_read_at: null, has_unread: true },
            { channel_id: "channel-design", last_read_at: null, has_unread: true },
          ]),
        ),
      ],
    },
  },
  play: playForImplementation(async () => {
    await expect(await screen.findAllByRole("img", { name: "未読メッセージあり" })).toHaveLength(2);
    await expect(screen.getByRole("img", { name: "アーカイブ済み" })).toBeInTheDocument();
  }),
};

const threadRoot = storyMessages.find((message) => message.id === "message-thread-root")!;
const threadEntry = {
  id: threadRoot.id,
  channel_id: threadRoot.channel_id,
  content: threadRoot.content,
  thread_archive_at: null as string | null,
  reply_count: storyMessages.filter((message) => message.parent_id === threadRoot.id).length,
};

export const Threads: Story = {
  parameters: {
    a11y: { test: "error" },
    msw: {
      handlers: [
        http.get("*/api/threads", () =>
          HttpResponse.json({ items: [threadEntry], next_cursor: null }),
        ),
      ],
    },
  },
  play: playForImplementation(async () => {
    await expect(await screen.findByRole("link", { name: /リリースノートの下書き/ })).toBeVisible();
  }),
};

export const ArchivedThreads: Story = {
  parameters: {
    a11y: { test: "error" },
    msw: {
      handlers: [
        http.get("*/api/threads", ({ request }) => {
          const archived = new URL(request.url).searchParams.get("archived") === "true";
          return HttpResponse.json({
            items: archived
              ? [{ ...threadEntry, thread_archive_at: "2026-01-10T09:05:00.000Z" }]
              : [],
            next_cursor: null,
          });
        }),
      ],
    },
  },
  play: playForImplementation(async () => {
    await userEvent.click(
      (await screen.findAllByRole("button", { name: /アーカイブ済みスレッド/ }))[0]!,
    );
    await expect(await screen.findByRole("link", { name: /リリースノートの下書き/ })).toBeVisible();
    await expect(screen.queryByRole("button", { name: /スレッドを再開/ })).not.toBeInTheDocument();
  }),
};

const olderRoot = storyMessages.find((message) => message.id === "message-hello")!;
const olderReply = {
  ...storyMessages.find((message) => message.parent_id === threadRoot.id)!,
  id: "message-older-thread-reply",
  parent_id: olderRoot.id,
};
const olderEntry = {
  ...threadEntry,
  id: olderRoot.id,
  content: olderRoot.content,
  reply_count: 1,
  thread_archive_at: "2026-01-01T00:00:00Z",
};

export const ArchivePages: Story = {
  parameters: {
    a11y: { test: "error" },
    collections: { ...storyCollections, messages: [...storyMessages, olderReply] },
    msw: {
      handlers: [
        http.get("*/api/threads", ({ request }) => {
          const params = new URL(request.url).searchParams;
          if (!params.has("archived")) return HttpResponse.json({ items: [], next_cursor: null });
          return HttpResponse.json(
            params.has("before")
              ? { items: [olderEntry], next_cursor: null }
              : {
                  items: [{ ...threadEntry, thread_archive_at: "2026-01-10T00:00:00Z" }],
                  next_cursor: "older",
                },
          );
        }),
      ],
    },
  },
  play: playForImplementation(async () => {
    await userEvent.click(
      (await screen.findAllByRole("button", { name: "アーカイブ済みスレッド" }))[0]!,
    );
    await userEvent.click(await screen.findByRole("button", { name: "さらに表示" }));
    await expect(
      await screen.findByRole("link", { name: (name) => name.includes(olderRoot.content) }),
    ).toBeVisible();
    await expect(screen.queryByRole("button", { name: "さらに表示" })).not.toBeInTheDocument();
  }),
};

let archiveFailed = false;
export const ArchiveRetry: Story = {
  beforeEach: () => {
    archiveFailed = false;
  },
  parameters: {
    a11y: { test: "error" },
    msw: {
      handlers: [
        http.get("*/api/threads", ({ request }) => {
          if (!new URL(request.url).searchParams.has("archived"))
            return HttpResponse.json({ items: [], next_cursor: null });
          if (!archiveFailed) {
            archiveFailed = true;
            return new HttpResponse(null, { status: 503 });
          }
          return HttpResponse.json({
            items: [{ ...threadEntry, thread_archive_at: "2026-01-10T00:00:00Z" }],
            next_cursor: null,
          });
        }),
      ],
    },
  },
  play: playForImplementation(async () => {
    await userEvent.click(
      (await screen.findAllByRole("button", { name: "アーカイブ済みスレッド" }))[0]!,
    );
    await expect(await screen.findByText("スレッド一覧を取得できませんでした。")).toBeVisible();
    await userEvent.click(screen.getByRole("button", { name: "再試行" }));
    await expect(await screen.findByRole("link", { name: /リリースノートの下書き/ })).toBeVisible();
  }),
};

export const EmptyThreadTitle: Story = {
  parameters: {
    a11y: { test: "error" },
    collections: {
      ...storyCollections,
      messages: storyMessages.map((message) =>
        message.id === threadRoot.id ? { ...message, content: "" } : message,
      ),
    },
    msw: {
      handlers: [
        http.get("*/api/threads", () =>
          HttpResponse.json({ items: [{ ...threadEntry, content: "" }], next_cursor: null }),
        ),
      ],
    },
  },
  play: playForImplementation(async () => {
    await expect(await screen.findByRole("link", { name: /無題のスレッド/ })).toBeVisible();
  }),
};

export const MobileThreads: Story = { ...Threads, globals: { viewport: { value: "mobile" } } };

const dm = {
  ...storyChannels[0],
  id: "dm-sidebar",
  channel_name: "dm:user-alice:user-bob",
  is_dm: true,
};
export const Sections: Story = {
  args: { channels: [...storyChannels, dm] },
  parameters: {
    a11y: { test: "error" },
    collections: {
      ...storyCollections,
      channels: [...storyChannels, dm],
      channelUsers: [
        ...storyChannelUsers,
        { channel_id: dm.id, user_id: "user-alice", joined_at: dm.created_at },
        { channel_id: dm.id, user_id: "user-bob", joined_at: dm.created_at },
      ],
    },
  },
  play: playForImplementation(async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const regular = await canvas.findByRole("button", { name: "チャンネル" });
    const direct = await canvas.findByRole("button", { name: "ダイレクトメッセージ" });
    const regularLink = await canvas.findByRole("link", { name: "general" });
    const directLink = await canvas.findByRole("link", { name: "Bob" });
    await expect(regularLink.getBoundingClientRect().height).toBe(
      directLink.getBoundingClientRect().height,
    );
    await userEvent.click(regular);
    await expect(regular).toHaveAttribute("aria-expanded", "false");
    await expect(regularLink).not.toBeVisible();
    await expect(directLink).toBeVisible();
    await userEvent.keyboard("{Enter}");
    await expect(regularLink).toBeVisible();
    await userEvent.click(direct);
    await expect(directLink).not.toBeVisible();
    await expect(regularLink).toBeVisible();
    await userEvent.click(direct);
    await expect(directLink).toBeVisible();
  }),
};
export const MobileSections: Story = { ...Sections, globals: { viewport: { value: "mobile" } } };
