import { playForImplementation } from "../../../../.storybook/play-for-implementation";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, fn, screen, spyOn, userEvent, waitFor } from "storybook/test";
import { delay, http, HttpResponse } from "msw";
import { useRef, type ComponentProps } from "react";

import { StoryShell } from "../../../../.storybook/shell";
import {
  storyChannels,
  storyNotifications,
  storyUsers,
  storyDmChannels,
  storyDmMembers,
  storyDmNotifications,
} from "../../../../.storybook/fixtures";
import type { ChannelRow, UserRow } from "../../../lib/electric/types";
import { NotificationCard } from "./NotificationCard";
import type { NotificationGroup } from "./groupNotifications";

const users = new Map<string, UserRow>(storyUsers.map((user) => [user.id, user]));
const channels = new Map<string, ChannelRow>(storyChannels.map((channel) => [channel.id, channel]));

function group(notificationId: string): NotificationGroup {
  const notification = storyNotifications.find((row) => row.id === notificationId)!;
  return {
    key: notification.id,
    latest: notification,
    notifications: [notification],
    actorIds: [notification.actor_id],
  };
}

function NotificationCardDemo(props: Omit<ComponentProps<typeof NotificationCard>, "scrollRoot">) {
  const scrollRoot = useRef<HTMLDivElement>(null);
  return (
    <StoryShell>
      <div ref={scrollRoot} className="w-full bg-background">
        <ul>
          <NotificationCard {...props} scrollRoot={scrollRoot} />
        </ul>
      </div>
    </StoryShell>
  );
}

const meta = {
  title: "Features/Notifications/Row",
  component: NotificationCard,
  tags: ["autodocs"],
  parameters: { a11y: { test: "error" } },
  argTypes: {
    channel: { control: false },
    users: { control: false },
  },
  args: {
    group: group("notification-mention"),
    channel: channels.get("channel-general"),
    users,
    onRead: fn(),
    scrollRoot: { current: null },
  },
  render: (args) => <NotificationCardDemo {...args} />,
} satisfies Meta<typeof NotificationCard>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Mention: Story = {
  play: playForImplementation(async () => {
    await expect(await screen.findByText(/今日のリリースは 18:00/)).toBeInTheDocument();
  }),
};

export const Reply: Story = {
  args: {
    group: group("notification-reply"),
  },
};

export const Reaction: Story = {
  args: {
    group: group("notification-reaction"),
  },
};

export const Read: Story = {
  args: {
    group: group("notification-reaction-read"),
  },
};

export const DirectMessage: Story = {
  args: {
    channel: storyDmChannels[0],
    group: {
      key: "notification-dm",
      latest: storyDmNotifications[0],
      notifications: [storyDmNotifications[0]],
      actorIds: ["user-bob"],
    },
  },
  play: playForImplementation(async () => {
    await expect(await screen.findByText("Bob", { selector: "p > span" })).toBeVisible();
    await expect(screen.queryByText(/#dm:/)).not.toBeInTheDocument();
    await expect(await screen.findByRole("link", { name: /個別に確認/ })).toHaveAttribute(
      "href",
      expect.stringContaining("/channels/channel-dm?"),
    );
  }),
};
export const GroupDirectMessage: Story = {
  args: {
    channel: storyDmChannels[1],
    members: storyDmMembers.filter((row) => row.channel_id === storyDmChannels[1].id),
    group: {
      key: "notification-dm-group",
      latest: storyDmNotifications[1],
      notifications: [storyDmNotifications[1]],
      actorIds: ["user-bob"],
    },
  },
  play: playForImplementation(async () => {
    await expect(await screen.findByText("Admin, Bob, Carol +1")).toBeVisible();
    await expect(screen.queryByText(/dm:group:/)).not.toBeInTheDocument();
  }),
};
export const DmMetadataPending: Story = {
  ...DirectMessage,
  args: { ...DirectMessage.args, users: new Map() },
  play: playForImplementation(async () => {
    await expect(
      await screen.findByText("ダイレクトメッセージ", { selector: "p > span" }),
    ).toBeVisible();
    await expect(screen.queryByText(/#dm:/)).not.toBeInTheDocument();
  }),
};

export const GroupedReactions: Story = {
  args: {
    group: {
      ...group("notification-reaction"),
      actorIds: ["user-bob", "user-carol"],
      notifications: [
        group("notification-reaction").latest,
        {
          ...group("notification-reaction").latest,
          id: "notification-reaction-carol",
          actor_id: "user-carol",
        },
      ],
    },
  },
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/notifications/notification-reaction/context", () =>
          HttpResponse.json({
            message: {
              id: "message-thread-root",
              user_id: "user-alice",
              content: "確認をお願いします。 https://example.com/release",
            },
            reactions: [
              { user_id: "user-bob", emoji: "👀" },
              { user_id: "user-carol", emoji: "👀" },
            ],
          }),
        ),
      ],
    },
  },
  play: playForImplementation(async () => {
    await expect(await screen.findByText("Bob さんほか1人")).toBeVisible();
    await expect(await screen.findByLabelText("👀 2")).toBeVisible();
    await expect(screen.getByRole("link", { name: "example.com/release" })).toHaveAttribute(
      "target",
      "_blank",
    );
    const external = screen.getByRole("link", { name: "example.com/release" });
    const bounds = external.getBoundingClientRect();
    await expect(
      document.elementFromPoint(bounds.x + bounds.width / 2, bounds.y + bounds.height / 2),
    ).toBe(external);
  }),
};
export const Loading: Story = {
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/notifications/:id/context", async () => {
          await delay("infinite");
          return HttpResponse.json({ message: null, reactions: [] });
        }),
      ],
    },
  },
  play: playForImplementation(async () => {
    await expect(await screen.findByText("読み込み中…")).toBeVisible();
  }),
};
export const Unavailable: Story = {
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/notifications/:id/context", () =>
          HttpResponse.json({ message: null, reactions: [] }),
        ),
      ],
    },
  },
  play: playForImplementation(async () => {
    await expect(await screen.findByText("メッセージが削除されたか、閲覧できません")).toBeVisible();
    await expect(screen.queryByRole("link")).not.toBeInTheDocument();
  }),
};
export const LongContentMobile: Story = {
  globals: { viewport: { value: "mobile" } },
  args: {
    channel: { ...storyChannels[0], channel_name: "長いチャンネル名".repeat(12) },
    users: new Map(
      storyUsers.map((user) => [
        user.id,
        user.id === "user-bob" ? { ...user, display_name: "長い名前".repeat(20) } : user,
      ]),
    ),
  },
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/notifications/:id/context", () =>
          HttpResponse.json({
            message: {
              id: "long",
              user_id: "user-bob",
              content: `**確認をお願いします** https://example.com/${"long-path/".repeat(35)}`,
            },
            reactions: [],
          }),
        ),
      ],
    },
  },
  play: playForImplementation(async () => {
    await screen.findByText("確認をお願いします", { selector: "strong" });
    await waitFor(() =>
      expect(document.documentElement.scrollWidth).toBe(document.documentElement.clientWidth),
    );
    const quote = document.querySelector("blockquote")!;
    await expect(quote.scrollWidth).toBeLessThanOrEqual(quote.clientWidth);
  }),
};

export const NativeInteractions: Story = {
  play: playForImplementation(async () => {
    const user = userEvent.setup();
    const external = await screen.findByRole("link", { name: "example.com/docs" });
    const preview = external.closest(".mfm-body")!;
    const time = document.querySelector("time")!;
    const timeBounds = time.getBoundingClientRect();
    await expect(
      document
        .elementFromPoint(timeBounds.x + timeBounds.width / 2, timeBounds.y + timeBounds.height / 2)
        ?.closest("time"),
    ).toBe(time);
    const text = Array.from(preview.childNodes).find(
      (node) => node.nodeType === Node.TEXT_NODE && (node.textContent?.length ?? 0) > 10,
    )!;
    const range = document.createRange();
    range.setStart(text, 1);
    range.setEnd(text, 8);
    const bounds = range.getBoundingClientRect();
    await expect(
      document
        .elementFromPoint(bounds.x + bounds.width / 2, bounds.y + bounds.height / 2)
        ?.closest(".mfm-body"),
    ).toBe(preview);
    const destination = screen.getByRole("link", { name: /^メンション · #general/ });
    const navigate = fn((event: Event) => event.preventDefault());
    destination.addEventListener("click", navigate);
    const selection = window.getSelection()!;
    try {
      selection.removeAllRanges();
      selection.addRange(range);
      await expect(selection.toString()).not.toBe("");
      preview.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      await expect(navigate).not.toHaveBeenCalled();
      selection.removeAllRanges();
      await user.click(preview);
      await expect(navigate).toHaveBeenCalledTimes(1);
      const open = spyOn(window, "open").mockReturnValue(null);
      try {
        await user.keyboard("{Control>}");
        await user.click(preview);
        await expect(open).toHaveBeenCalledWith(
          (destination as HTMLAnchorElement).href,
          "_blank",
          "noopener,noreferrer",
        );
      } finally {
        await user.keyboard("{/Control}");
        open.mockRestore();
      }
    } finally {
      selection.removeAllRanges();
      destination.removeEventListener("click", navigate);
    }
  }),
};
