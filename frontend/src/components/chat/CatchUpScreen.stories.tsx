import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, screen, userEvent, waitFor, within } from "storybook/test";
import { delay, http, HttpResponse } from "msw";

import { StoryShell } from "../../../.storybook/shell";
import { storySurfaces } from "../../../.storybook/story-surfaces";
import { storyChannels, storyCollections, storyUsers } from "../../../.storybook/fixtures";
import { CatchUpScreen } from "./CatchUpScreen";

const channels = [
  { ...storyChannels[0], id: "catchup-a", channel_name: "general", is_dm: false },
  { ...storyChannels[0], id: "catchup-b", channel_name: "random", is_dm: false },
];

const message = (id: string, channelId: string, content: string, seconds: number) => ({
  id,
  user_id: "user-bob",
  channel_id: channelId,
  content,
  created_at: `2026-01-02T09:00:0${seconds}Z`,
  updated_at: null,
  deleted_at: null,
  parent_id: null,
});

const collections = {
  ...storyCollections,
  channels,
  messages: [
    message("catchup-a-1", "catchup-a", "今日のリリースは 18:00 です。", 1),
    message("catchup-a-2", "catchup-a", "レビューをお願いします。", 2),
    message("catchup-b-1", "catchup-b", "週末の予定を確認してください。", 3),
    // 既読にしても境界が動かないことを確認する、ページを開いた後の新着。
    {
      ...message("catchup-a-3", "catchup-a", "新しいメッセージは次回に残ります。", 4),
    },
  ],
  users: storyUsers,
};

const unreadStates = [
  {
    channel_id: "catchup-a",
    last_read_at: null,
    has_unread: true,
    unread_count: 3,
    unread_count_truncated: false,
    first_unread_at: "2026-01-02 09:00:01",
  },
  {
    channel_id: "catchup-b",
    last_read_at: null,
    has_unread: true,
    unread_count: 2,
    unread_count_truncated: false,
    first_unread_at: "2026-01-02 09:00:03",
  },
];

const readHandler = http.post("*/api/channels/:id/read-state", () =>
  HttpResponse.json({ data: { has_unread: false }, txid: null }),
);
const listHandler = http.get("*/api/channel-read-states", () => HttpResponse.json(unreadStates));

const meta = {
  title: "Pages/CatchUp",
  component: CatchUpScreen,
  tags: ["autodocs"],
  parameters: {
    a11y: { test: "error" },
    collections,
    router: { initialEntries: ["/catch-up"] },
    msw: { handlers: [listHandler, readHandler] },
  },
  decorators: [
    (Story) => (
      <StoryShell selectedChannelId={null} initialPane="chat">
        <div className={storySurfaces["Pages/Notifications"]}>
          <Story />
        </div>
      </StoryShell>
    ),
  ],
} satisfies Meta<typeof CatchUpScreen>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};

export const LongUnread: Story = {
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/channel-read-states", () =>
          HttpResponse.json([
            { ...unreadStates[0], unread_count: 1000, unread_count_truncated: true },
            unreadStates[1],
          ]),
        ),
        readHandler,
      ],
    },
  },
};

export const Empty: Story = {
  parameters: {
    msw: {
      handlers: [http.get("*/api/channel-read-states", () => HttpResponse.json([])), readHandler],
    },
  },
};

export const Loading: Story = {
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/channel-read-states", async () => {
          await delay("infinite");
          return HttpResponse.json([]);
        }),
        readHandler,
      ],
    },
  },
};

export const LoadError: Story = {
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/channel-read-states", () =>
          HttpResponse.json({ message: "failed" }, { status: 500 }),
        ),
        readHandler,
      ],
    },
  },
};

export const Complete: Story = {
  play: async () => {
    await screen.findByText("レビューをお願いします。");
    const markRead = await screen.findByRole("button", { name: "既読にする" });
    await userEvent.click(markRead);
    await waitFor(() => expect(screen.getByText("2 / 2")).toBeVisible());
    const keepUnread = await screen.findByRole("button", { name: "未読のまま次へ" });
    await userEvent.click(keepUnread);
    await expect(await screen.findByText("キャッチアップ完了")).toBeVisible();
  },
};

export const WithReactions: Story = {
  parameters: {
    collections: {
      ...collections,
      reactions: [
        {
          id: "catchup-reaction-bob",
          message_id: "catchup-a-2",
          user_id: "user-bob",
          emoji: "👍",
          created_at: "2026-01-02T09:00:05Z",
        },
        {
          id: "catchup-reaction-alice",
          message_id: "catchup-a-2",
          user_id: "user-alice",
          emoji: "🎉",
          created_at: "2026-01-02T09:00:06Z",
        },
      ],
    },
    msw: {
      handlers: [
        listHandler,
        readHandler,
        http.get("*/api/emojis", () => HttpResponse.json([])),
        http.post("*/api/messages/:id/reactions", async ({ request, params }) => {
          const body = (await request.json()) as { id?: string; emoji?: string };
          return HttpResponse.json({
            data: {
              id: body.id ?? "catchup-reaction-new",
              message_id: String(params.id),
              user_id: "user-alice",
              emoji: body.emoji ?? "😀",
              created_at: "2026-01-02T09:00:10Z",
            },
            txid: null,
          });
        }),
        http.delete("*/api/messages/:id/reactions", () =>
          HttpResponse.json({ data: { id: "deleted" }, txid: null }),
        ),
      ],
    },
  },
  play: async () => {
    await screen.findByText("レビューをお願いします。");
    const group = await screen.findByRole("group", { name: "リアクション" });

    const ownChip = within(group).getByRole("button", {
      name: "🎉 1 件（自分もリアクション済み）",
    });
    await expect(ownChip).toHaveAttribute("aria-pressed", "true");
    await userEvent.click(ownChip);
    await waitFor(() =>
      expect(within(group).queryByRole("button", { name: /🎉/ })).not.toBeInTheDocument(),
    );

    const otherChip = within(group).getByRole("button", { name: "👍 1 件" });
    await expect(otherChip).toHaveAttribute("aria-pressed", "false");
    await userEvent.click(otherChip);
    await waitFor(() =>
      expect(
        within(group).getByRole("button", { name: "👍 2 件（自分もリアクション済み）" }),
      ).toHaveAttribute("aria-pressed", "true"),
    );

    await userEvent.click(within(group).getByRole("button", { name: "リアクションを追加" }));
    const dialog = await screen.findByRole("dialog", {}, { timeout: 5000 });
    const cells = await within(dialog).findAllByRole("gridcell", {}, { timeout: 5000 });
    const targetCell =
      cells.find(
        (cell) => cell.textContent && cell.textContent !== "👍" && cell.textContent !== "🎉",
      ) ?? cells[0]!;
    const picked = targetCell.textContent ?? "😀";
    await userEvent.click(targetCell);

    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    await expect(
      await within(group).findByRole("button", {
        name: `${picked} 1 件（自分もリアクション済み）`,
      }),
    ).toHaveAttribute("aria-pressed", "true");
    await expect(screen.getByText("1 / 2")).toBeVisible();
    await expect(screen.getByRole("button", { name: "既読にする" })).toBeEnabled();
  },
};

export const Dark: Story = { ...Default, globals: { theme: "dark" } };

export const Mobile: Story = { ...Default, globals: { viewport: { value: "mobile" } } };
