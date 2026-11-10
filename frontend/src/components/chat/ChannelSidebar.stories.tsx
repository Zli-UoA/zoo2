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
    msw: { handlers: [http.get("*/api/threads", () => HttpResponse.json([threadEntry]))] },
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
        http.get("*/api/threads", () =>
          HttpResponse.json([{ ...threadEntry, thread_archive_at: "2026-01-10T09:05:00.000Z" }]),
        ),
      ],
    },
  },
  play: playForImplementation(async () => {
    await userEvent.click(await screen.findByRole("button", { name: /アーカイブ済みスレッド/ }));
    await expect(await screen.findByRole("link", { name: /リリースノートの下書き/ })).toBeVisible();
    await expect(screen.queryByRole("button", { name: /スレッドを再開/ })).not.toBeInTheDocument();
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
