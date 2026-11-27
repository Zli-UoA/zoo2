import { playForImplementation } from "../../../.storybook/play-for-implementation";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { http, HttpResponse } from "msw";
import { expect, screen, userEvent, waitFor, within } from "storybook/test";

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
          notification_level: "mentions",
        },
      ],
    },
    msw: {
      handlers: [
        http.get("*/api/channel-read-states", () =>
          HttpResponse.json([
            {
              channel_id: "00000000-0000-4000-8000-000000000001",
              last_read_at: null,
              has_unread: true,
            },
            { channel_id: "channel-design", last_read_at: null, has_unread: true },
          ]),
        ),
      ],
    },
  },
  play: playForImplementation(async () => {
    await expect(await screen.findAllByRole("img", { name: "未読メッセージあり" })).toHaveLength(2);
    // アーカイブ済みは参加済みでもサイドバーに出さない（「チャンネルを探す」から開く）。
    await expect(screen.queryByRole("img", { name: "アーカイブ済み" })).not.toBeInTheDocument();
    await expect(screen.queryByRole("link", { name: /old-announcements/ })).not.toBeInTheDocument();
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

const nsfw = {
  ...storyChannels[1],
  id: "channel-nsfw",
  channel_name: "nsfw",
  is_nsfw: true,
};

const pinnedAt = "2026-01-03 09:00:00.000";
const pinnedRows = [
  {
    user_id: "user-alice",
    channel_id: "channel-design",
    position: 0,
    created_at: pinnedAt,
    updated_at: pinnedAt,
  },
  {
    user_id: "user-alice",
    channel_id: dm.id,
    position: 1,
    created_at: pinnedAt,
    updated_at: pinnedAt,
  },
  {
    user_id: "user-alice",
    channel_id: nsfw.id,
    position: 2,
    created_at: pinnedAt,
    updated_at: pinnedAt,
  },
  // アーカイブ中のピンはサーバーに残るが、サイドバーには出さない。
  {
    user_id: "user-alice",
    channel_id: "channel-archive",
    position: 3,
    created_at: pinnedAt,
    updated_at: pinnedAt,
  },
];

const pinnedCollections = {
  ...storyCollections,
  channels: [...storyChannels, dm, nsfw],
  channelUsers: [
    ...storyChannelUsers,
    { channel_id: dm.id, user_id: "user-alice", joined_at: dm.created_at },
    { channel_id: dm.id, user_id: "user-bob", joined_at: dm.created_at },
    { channel_id: nsfw.id, user_id: "user-alice", joined_at: nsfw.created_at },
  ],
  userPinnedChannels: pinnedRows,
};

const pinnedApi = http.get("*/api/channel-pins", () =>
  HttpResponse.json({ pins: pinnedRows, version: 3 }),
);

const pinnedReorderRequests: unknown[] = [];
const pinnedReorderApi = http.patch("*/api/channel-pins/order", async ({ request }) => {
  pinnedReorderRequests.push(await request.json());
  return HttpResponse.json({
    data: {
      version: 4,
      pins: [
        { ...pinnedRows[1], position: 0 },
        { ...pinnedRows[0], position: 1 },
        { ...pinnedRows[2], position: 2 },
        { ...pinnedRows[3], position: 3 },
      ],
    },
    txid: "12",
  });
});

export const Pinned: Story = {
  args: { channels: [...storyChannels, dm, nsfw] },
  parameters: {
    // メニューを開くと Base UI のフォーカスガードが aria-hidden で focusable になる。
    // メニュー操作自体は play で確認し、静的な a11y は対象外にする。
    a11y: { test: "todo" },
    collections: pinnedCollections,
    msw: { handlers: [pinnedApi, pinnedReorderApi] },
  },
  play: playForImplementation(async ({ canvasElement }) => {
    pinnedReorderRequests.length = 0;
    const canvas = within(canvasElement);
    await expect(await canvas.findByRole("button", { name: "ピン留め" })).toBeVisible();
    const section = await canvas.findByRole("region", { name: "ピン留め" });
    const pinned = within(section);
    // ピン留めの節では、通常チャンネルと DM が保存した順に並ぶ。
    await expect(pinned.getByRole("link", { name: "design" })).toBeVisible();
    await expect(pinned.getByRole("link", { name: "Bob" })).toBeVisible();
    await expect(pinned.getByRole("link", { name: /nsfw/ })).toBeVisible();
    await expect(pinned.getAllByText("NSFW")).toHaveLength(1);
    const initialLinks = pinned.getAllByRole("link");
    await expect(initialLinks).toHaveLength(3);
    // 保存した順（design → Bob）で、通常の一覧や並び順の既定値に引きずられない。
    await expect(initialLinks[0]).toHaveTextContent("design");
    await expect(initialLinks[1]).toHaveTextContent("Bob");
    // アーカイブ中のピンは表示しない。
    await expect(canvas.queryAllByRole("link", { name: "old-announcements" })).toHaveLength(0);

    // 行メニューの「下へ移動」で PATCH が呼ばれ、返った並びを表示する。
    await userEvent.click(await pinned.findByRole("button", { name: "design のチャンネル操作" }));
    await userEvent.click(await screen.findByRole("menuitem", { name: "下へ移動" }));
    await waitFor(() => expect(pinnedReorderRequests).toHaveLength(1));
    await expect(pinnedReorderRequests[0]).toMatchObject({
      channel_ids: [dm.id, "channel-design", nsfw.id],
    });
    await waitFor(async () => {
      const links = pinned.getAllByRole("link");
      await expect(links[0]).toHaveTextContent("Bob");
      await expect(links[1]).toHaveTextContent("design");
    });
  }),
};

export const MobilePinned: Story = {
  ...Pinned,
  globals: { viewport: { value: "mobile" } },
};

// スレッドのあるチャンネルをピン留めすると、ピン留めの行の下にも同じスレッドのツリーを出す
// （通常のチャンネル一覧の行のツリーはそのまま残る）。DM にはツリーを出さない。
export const PinnedThreads: Story = {
  parameters: {
    a11y: { test: "error" },
    collections: {
      ...storyCollections,
      userPinnedChannels: [
        {
          user_id: "user-alice",
          channel_id: threadRoot.channel_id,
          position: 0,
          created_at: pinnedAt,
          updated_at: pinnedAt,
        },
      ],
    },
    msw: {
      handlers: [
        http.get("*/api/channel-pins", () =>
          HttpResponse.json({
            pins: [
              {
                user_id: "user-alice",
                channel_id: threadRoot.channel_id,
                position: 0,
                created_at: pinnedAt,
                updated_at: pinnedAt,
              },
            ],
            version: 1,
          }),
        ),
        http.get("*/api/threads", () =>
          HttpResponse.json({ items: [threadEntry], next_cursor: null }),
        ),
      ],
    },
  },
  play: playForImplementation(async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const section = await canvas.findByRole("region", { name: "ピン留め" });
    await expect(
      await within(section).findByRole("link", { name: /リリースノートの下書き/ }),
    ).toBeVisible();
    // 通常のチャンネル一覧にも、同じスレッドが残る。
    await expect(
      await canvas.findAllByRole("link", { name: /リリースノートの下書き/ }),
    ).toHaveLength(2);
  }),
};

export const PinnedReorderError: Story = {
  args: { channels: [...storyChannels, dm] },
  parameters: {
    a11y: { test: "error" },
    collections: pinnedCollections,
    msw: {
      handlers: [
        http.get("*/api/channel-pins", () =>
          HttpResponse.json({ error: "server error" }, { status: 500 }),
        ),
      ],
    },
  },
  play: playForImplementation(async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(await canvas.findByRole("alert")).toBeVisible();
    // 取得に失敗しても、Electric の行があればピンは表示する。
    await expect((await canvas.findAllByRole("link", { name: "design" })).length).toBeGreaterThan(
      0,
    );
  }),
};

export const SelectedArchivedThread: Story = {
  decorators: [
    (Story) => (
      <StoryShell selectedChannelId={threadRoot.channel_id} initialThreadRootId={threadRoot.id}>
        <Story />
      </StoryShell>
    ),
  ],
  parameters: {
    a11y: { test: "error" },
    collections: {
      ...storyCollections,
      messages: storyMessages.map((message) =>
        message.id === threadRoot.id
          ? { ...message, thread_archive_at: "2020-01-01T00:00:00Z" }
          : message,
      ),
    },
    msw: {
      handlers: [
        http.get("*/api/threads", () => HttpResponse.json({ items: [], next_cursor: null })),
      ],
    },
  },
  play: playForImplementation(async () => {
    await expect(await screen.findByRole("link", { name: "general" })).toBeVisible();
    await expect(
      screen.queryByRole("link", { name: /リリースノートの下書き/ }),
    ).not.toBeInTheDocument();
    await expect(
      screen.queryByRole("button", { name: /アーカイブ済みスレッド/ }),
    ).not.toBeInTheDocument();
    await expect(screen.queryByRole("searchbox")).not.toBeInTheDocument();
  }),
};

export const SelectedPostWithoutReplies: Story = {
  decorators: SelectedArchivedThread.decorators,
  parameters: {
    a11y: { test: "error" },
    collections: {
      ...storyCollections,
      messages: storyMessages
        .filter((message) => message.parent_id !== threadRoot.id)
        .map((message) =>
          message.id === threadRoot.id
            ? {
                ...message,
                thread_archive_at: null,
                display_data: { ...message.display_data, reply_count: 0 },
              }
            : message,
        ),
    },
    msw: {
      handlers: [
        http.get("*/api/threads", () => HttpResponse.json({ items: [], next_cursor: null })),
      ],
    },
  },
  play: playForImplementation(async () => {
    await expect(await screen.findByRole("link", { name: "general" })).toBeVisible();
    await expect(
      screen.queryByRole("link", { name: /リリースノートの下書き/ }),
    ).not.toBeInTheDocument();
  }),
};
