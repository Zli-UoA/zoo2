import { activityPubNotifications } from "../../../.storybook/activitypub-fixtures";
import { http, HttpResponse } from "msw";
import { HomeScreen } from "../home/HomeScreen";
import { homeHandlers } from "../../../.storybook/home-fixtures";
import { playForImplementation } from "../../../.storybook/play-for-implementation";
import { BubbleChatScreen } from "../chat/BubbleChatScreen";
import { bubbleChatHandlers } from "../../../.storybook/bubble-fixtures";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, screen, userEvent, waitFor } from "storybook/test";

import { storyCollections, storyNotifications } from "../../../.storybook/fixtures";
import { ChannelScreen } from "../chat/ChannelScreen";
import { NotificationsScreen } from "../chat/NotificationsScreen";
import { WelcomePanel } from "../chat/WelcomePanel";
import { AppShell } from "./AppShell";

const meta = {
  title: "Pages/Shell",
  component: AppShell,
  tags: ["autodocs"],
  parameters: { a11y: { test: "todo" } },
  args: {
    children: <WelcomePanel />,
  },
} satisfies Meta<typeof AppShell>;

export default meta;
type Story = StoryObj<typeof meta>;

async function expectAlignedHeaders() {
  const railHeader = screen.getByRole("navigation").querySelector("header")!;
  const mainHeader = screen.getByRole("main").querySelector("header")!;
  await expect(mainHeader.getBoundingClientRect().top).toBe(railHeader.getBoundingClientRect().top);
  await expect(mainHeader.getBoundingClientRect().bottom).toBe(
    railHeader.getBoundingClientRect().bottom,
  );
}

export const Welcome: Story = {
  play: playForImplementation(async () => {
    await expect(screen.getByLabelText("チャンネル", { selector: "aside" })).toBeVisible();
  }),
};

export const Channel: Story = {
  args: {
    children: <ChannelScreen channelId="channel-general" />,
  },
  parameters: {
    router: { initialEntries: ["/channels/channel-general"] },
  },
  play: playForImplementation(async () => {
    await expect(await screen.findByText(/今日のリリースは 18:00/)).toBeInTheDocument();
    await expect(screen.getByLabelText("チャンネル", { selector: "aside" })).toBeVisible();
    await expectAlignedHeaders();
  }),
};

export const MobileChannel: Story = {
  globals: { viewport: { value: "mobile" } },
  args: {
    children: <ChannelScreen channelId="channel-general" />,
  },
  parameters: {
    router: { initialEntries: ["/channels/channel-general"] },
  },
  play: playForImplementation(async () => {
    await screen.findByRole("heading", { name: "general" });
    await expectAlignedHeaders();
  }),
};

export const ChannelWithMembers: Story = {
  ...Channel,
  play: playForImplementation(async (context) => {
    await Channel.play?.(context);
    await userEvent.click(screen.getByRole("button", { name: /^メンバー / }));
    const membersHeader = (
      await screen.findByRole("heading", { name: "メンバー", level: 3 })
    ).closest("header")!;
    const railHeader = screen.getByRole("navigation").querySelector("header")!;
    await expect(membersHeader.getBoundingClientRect().bottom).toBe(
      railHeader.getBoundingClientRect().bottom,
    );
  }),
};

export const Notifications: Story = {
  args: {
    children: <NotificationsScreen />,
  },
  parameters: {
    a11y: { test: "error" },
    router: { initialEntries: ["/notifications"] },
  },
  play: playForImplementation(async () => {
    await waitFor(() => expect(screen.queryByRole("status")).not.toBeInTheDocument());
    await expect(screen.getByLabelText("チャンネル", { selector: "aside" })).not.toBeVisible();
    const main = screen.getByRole("main").getBoundingClientRect();
    const nav = screen.getByRole("navigation").getBoundingClientRect();
    await expect(main.x).toBe(nav.right);
    await expect(main.right).toBe(document.documentElement.clientWidth);
    await expectAlignedHeaders();
  }),
};

export const NotificationsSingleUnread: Story = {
  args: {
    children: <NotificationsScreen />,
  },
  parameters: {
    router: { initialEntries: ["/notifications"] },
    collections: {
      ...storyCollections,
      notifications: storyNotifications.map((notification, index) => ({
        ...notification,
        is_read: index !== 0,
      })),
    },
  },
};

export const Mobile: Story = {
  globals: { viewport: { value: "mobile" } },
  args: {
    children: <WelcomePanel />,
  },
};

export const MobileNotifications: Story = {
  ...Notifications,
  globals: { viewport: { value: "mobile" } },
};

export const DarkNotifications: Story = {
  ...Notifications,
  globals: { theme: "dark" },
};

export const BubbleChat: Story = {
  args: { children: <BubbleChatScreen /> },
  parameters: {
    a11y: { test: "error" },
    router: { initialEntries: ["/bubble"] },
    msw: { handlers: bubbleChatHandlers },
  },
  play: playForImplementation(async () => {
    await expect(
      await screen.findByRole("textbox", { name: "リアルタイムメッセージ入力" }),
    ).toBeVisible();
    await expect(screen.getByLabelText("チャンネル", { selector: "aside" })).not.toBeVisible();
    await expect(screen.getByRole("link", { name: "Bubble Chat" })).toHaveAttribute(
      "data-status",
      "active",
    );
    await expectAlignedHeaders();
  }),
};

export const MobileBubbleChat: Story = {
  ...BubbleChat,
  globals: { viewport: { value: "mobile" } },
};

export const MobileMembers: Story = {
  args: { children: <ChannelScreen channelId="channel-general" /> },
  globals: { viewport: { value: "mobile" } },
  parameters: {
    a11y: { test: "error" },
    router: { initialEntries: ["/channels/channel-general?members=true"] },
  },
  play: playForImplementation(async () => {
    await expect(await screen.findByRole("heading", { name: "メンバー", level: 1 })).toBeVisible();
    await expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    await expect(screen.getByLabelText("メインナビゲーション")).not.toBeVisible();
  }),
};

export const Home: Story = {
  args: { children: <HomeScreen /> },
  parameters: {
    a11y: { test: "error" },
    router: { initialEntries: ["/home"] },
    msw: { handlers: homeHandlers },
  },
  play: async () => {
    await expect(await screen.findByText(/Homeから最初の公開投稿です/)).toBeVisible();
    await expect(screen.getByRole("link", { name: "Home" })).toHaveAttribute(
      "data-status",
      "active",
    );
    await expect(screen.getByLabelText("チャンネル", { selector: "aside" })).not.toBeVisible();
    await expectAlignedHeaders();
    const main = screen.getByRole("main").getBoundingClientRect();
    const rail = screen.getByRole("navigation").getBoundingClientRect();
    await expect(main.x).toBe(rail.right);
    const button = screen.getByRole("button", { name: "投稿を書く" }).getBoundingClientRect();
    await expect(main.bottom - button.bottom).toBeGreaterThanOrEqual(19);
    await expect(main.bottom - button.bottom).toBeLessThanOrEqual(33);
    await expect(main.bottom).toBe(window.innerHeight);
  },
};
export const MobileHome: Story = { ...Home, globals: { viewport: { value: "mobile" } } };
export const DarkHome: Story = { ...Home, globals: { theme: "dark" } };

export const EmptyHome: Story = {
  ...Home,
  parameters: {
    ...Home.parameters,
    msw: { handlers: [http.get("*/api/home", () => HttpResponse.json([]))] },
  },
  play: async () => {
    await expect(await screen.findByText("まだ投稿がありません")).toBeVisible();
    const main = screen.getByRole("main").getBoundingClientRect();
    const button = screen.getByRole("button", { name: "投稿を書く" }).getBoundingClientRect();
    await expect(main.bottom - button.bottom).toBeGreaterThanOrEqual(19);
    await expect(main.bottom - button.bottom).toBeLessThanOrEqual(33);
    await expect(main.bottom).toBe(window.innerHeight);
  },
};
export const MobileEmptyHome: Story = { ...EmptyHome, globals: { viewport: { value: "mobile" } } };

export const FederatedUnread: Story = {
  ...Home,
  parameters: {
    ...Home.parameters,
    msw: {
      handlers: [
        ...homeHandlers,
        http.get("*/api/activitypub/notifications", () =>
          HttpResponse.json({ items: activityPubNotifications, unread_count: 3 }),
        ),
      ],
    },
  },
  play: async () => {
    await expect(await screen.findByRole("link", { name: "未読通知 6 件" })).toBeVisible();
  },
};
