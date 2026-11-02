import { blobcat, activityPubNotifications } from "../../../.storybook/activitypub-fixtures";
import {
  notificationComparisonText,
  notificationComparisonLabel,
  notificationComparisonMemberCount,
} from "../../../.storybook/notification-fixtures";
import { storySurfaces } from "../../../.storybook/story-surfaces";
import { playForImplementation } from "../../../.storybook/play-for-implementation";
import type { Decorator, Meta, StoryObj } from "@storybook/react-vite";
import { expect, screen, userEvent, waitFor, within } from "storybook/test";
import { delay, http, HttpResponse, ws } from "msw";
import { useLayoutEffect, type ReactNode } from "react";

import { StoryShell } from "../../../.storybook/shell";
import {
  storyCollections,
  storyNotifications,
  storyChannels,
  storyChannelUsers,
  storyDmChannels,
  storyDmMembers,
  storyDmNotifications,
} from "../../../.storybook/fixtures";
import { NotificationsScreen } from "./NotificationsScreen";
import { ChannelHeader } from "./ChannelHeader";
import { MessageBody } from "./messages/MessageRow";

// 通知カードの自動既読はフォーカス中だけ動く。Storybook では未読の見た目を
// 確認できるよう、この Story の間だけフォーカスを外す。
function UnreadPreview({ children }: { children: ReactNode }) {
  useLayoutEffect(() => {
    const original = document.hasFocus.bind(document);
    document.hasFocus = () => false;
    return () => {
      document.hasFocus = original;
    };
  }, []);
  return <>{children}</>;
}

function ForegroundPreview({ children }: { children: ReactNode }) {
  useLayoutEffect(() => {
    const original = document.hasFocus.bind(document);
    document.hasFocus = () => true;
    window.dispatchEvent(new Event("focus"));
    return () => {
      document.hasFocus = original;
    };
  }, []);
  return <>{children}</>;
}
const withForegroundPreview: Decorator = (Story) => (
  <ForegroundPreview>
    <Story />
  </ForegroundPreview>
);

const withUnreadPreview: Decorator = (Story) => (
  <UnreadPreview>
    <Story />
  </UnreadPreview>
);

// A missing spinner also matches the period before React commits its first render.
// Wait for the actual list and its visible state before asserting loading has ended.
async function readyNotificationsList() {
  const region = await screen.findByRole("region", { name: "通知" });
  const list = await within(region).findByRole("list", { name: "通知" });
  await waitFor(async () => {
    await expect(list).toBeVisible();
    await expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });
  return list;
}

const meta = {
  title: "Pages/Notifications",
  component: NotificationsScreen,
  tags: ["autodocs"],
  parameters: { a11y: { test: "error" } },
  play: playForImplementation(async () => {
    await screen.findByRole("region", { name: "通知" });
    await waitFor(() => expect(screen.queryByRole("status")).not.toBeInTheDocument());
  }),
  decorators: [
    (Story) => (
      <StoryShell selectedChannelId={null} initialPane="chat">
        <div className={storySurfaces["Pages/Notifications"]}>
          <Story />
        </div>
      </StoryShell>
    ),
  ],
} satisfies Meta<typeof NotificationsScreen>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {
  decorators: [withUnreadPreview],
  play: playForImplementation(async () => {
    await readyNotificationsList();
    await waitFor(() => expect(screen.queryByRole("status")).not.toBeInTheDocument());
    await expect(await screen.findByText(/今日のリリースは 18:00/)).toBeVisible();
    const list = screen.getByRole("list", { name: "通知" });
    const region = screen.getByRole("region", { name: "通知" });
    await expect(list.getBoundingClientRect().width).toBe(region.getBoundingClientRect().width);
    for (const row of Array.from(list.children))
      await expect(row.getBoundingClientRect().width).toBe(list.getBoundingClientRect().width - 32);
    await expect(within(list).getByRole("link", { name: "example.com/docs" })).toHaveAttribute(
      "target",
      "_blank",
    );
  }),
};

export const SingleUnread: Story = {
  parameters: {
    collections: {
      ...storyCollections,
      notifications: storyNotifications.map((notification, index) => ({
        ...notification,
        is_read: index !== 0,
      })),
    },
  },
};

export const Empty: Story = {
  parameters: {
    collections: { ...storyCollections, notifications: [] },
  },
};

export const Mobile: Story = {
  play: Default.play,
  globals: { viewport: { value: "mobile" } },
  decorators: [withUnreadPreview],
};

export const DirectMessages: Story = {
  decorators: [withUnreadPreview],
  parameters: {
    collections: {
      ...storyCollections,
      channels: [...storyChannels, ...storyDmChannels],
      channelUsers: [...storyChannelUsers, ...storyDmMembers],
      notifications: storyDmNotifications,
    },
  },
  play: playForImplementation(async () => {
    await readyNotificationsList();
    await waitFor(() => expect(screen.queryByRole("status")).not.toBeInTheDocument());
    await expect(await screen.findByText("Admin, Bob, Carol +1")).toBeVisible();
    await expect(await screen.findByText("Bob", { selector: "p > span" })).toBeVisible();
    await expect(screen.queryByText(/#dm:/)).not.toBeInTheDocument();
  }),
};

export const Dark: Story = { ...Default, globals: { theme: "dark" } };

export const MessageLayoutComparison: Story = {
  decorators: [withUnreadPreview],
  render: () => (
    <div className="grid h-full grid-cols-1 grid-rows-2 md:grid-cols-2 md:grid-rows-1">
      <section aria-label={notificationComparisonLabel}>
        <ChannelHeader
          channel={storyChannels[0]!}
          memberCount={notificationComparisonMemberCount}
        />
        <div className="p-3">
          <MessageBody content={notificationComparisonText} />
        </div>
      </section>
      <NotificationsScreen />
    </div>
  ),
  play: playForImplementation(async () => {
    await readyNotificationsList();
    await waitFor(() => expect(screen.queryByRole("status")).not.toBeInTheDocument());
    const notification = screen.getByRole("region", { name: "通知" });
    const message = screen.getByRole("region", { name: "メッセージ表示の比較用" });
    const notificationHeader = notification.querySelector("header")!;
    const messageHeader = message.querySelector("header")!;
    await expect(notificationHeader.getBoundingClientRect().height).toBe(
      messageHeader.getBoundingClientRect().height,
    );
    await expect(getComputedStyle(within(notification).getByRole("heading")).fontSize).toBe(
      getComputedStyle(within(message).getByRole("heading")).fontSize,
    );
    const notificationBody = getComputedStyle(notification.querySelector(".mfm-body")!);
    const messageBody = getComputedStyle(message.querySelector(".mfm-body")!);
    await expect(notificationBody.fontSize).toBe(messageBody.fontSize);
    await expect(notificationBody.lineHeight).toBe(messageBody.lineHeight);
    await expect(getComputedStyle(notification.querySelector("time")!).fontSize).toBe("11px");
  }),
};

export const MessageLayoutComparisonMobile: Story = {
  ...MessageLayoutComparison,
  globals: { viewport: { value: "mobile" } },
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
    await expect(await screen.findByRole("status")).toBeVisible();
    const list = within(screen.getByRole("region", { name: "通知" })).getByRole("list", {
      hidden: true,
    });
    await expect(list).toHaveAttribute("inert");
    await expect(list).not.toBeVisible();
    await waitFor(() => expect(list.querySelectorAll(":scope > [data-unread]")).toHaveLength(3));
  }),
};
// Both implementations hold the same real context response until their play has
// observed the initial mask. A timer cannot guarantee that observation on Wasm startup.
let releaseNotificationBodies = () => {};
let notificationBodies = Promise.resolve();
export const BodiesReadyTogether: Story = {
  beforeEach: () => {
    notificationBodies = new Promise<void>((resolve) => {
      releaseNotificationBodies = resolve;
    });
    return () => releaseNotificationBodies();
  },
  parameters: {
    fixtureControls: { releaseBodies: () => releaseNotificationBodies() },
    msw: {
      handlers: [
        http.get("*/api/notifications/:id/context", async ({ params }) => {
          await notificationBodies;
          return HttpResponse.json({
            message: {
              id: String(params.id),
              user_id: "user-bob",
              content: `本文 ${String(params.id)}`,
            },
            reactions: [],
          });
        }),
      ],
    },
  },
  play: playForImplementation(async () => {
    await expect(await screen.findByRole("status")).toBeVisible();
    await expect(
      within(screen.getByRole("region", { name: "通知" })).getByRole("list", { hidden: true }),
    ).not.toBeVisible();
    releaseNotificationBodies();
    await waitFor(() => expect(screen.queryByRole("status")).not.toBeInTheDocument());
    await expect(screen.getByRole("list", { name: "通知" })).toBeVisible();
    await expect(screen.getByText("本文 notification-mention")).toBeVisible();
    await expect(screen.getAllByText(/^本文 notification-/)).toHaveLength(4);
  }),
};

export const LoadingOnScroll: Story = {
  decorators: [withUnreadPreview],
  parameters: {
    collections: {
      ...storyCollections,
      notifications: Array.from({ length: 20 }, (_, index) => ({
        ...storyNotifications[0]!,
        id: `scroll-${index}`,
        message_id: `scroll-message-${index}`,
        created_at: new Date(Date.UTC(2026, 0, 2, 9, 0, 20 - index)).toISOString(),
      })),
    },
    msw: {
      handlers: [
        http.get("*/api/notifications/:id/context", async ({ params }) => {
          if (Number(String(params.id).slice("scroll-".length)) >= 10) await delay("infinite");
          return HttpResponse.json({
            message: {
              id: String(params.id),
              user_id: "user-bob",
              content: `本文 ${String(params.id)}`,
            },
            reactions: [],
          });
        }),
      ],
    },
  },
  play: playForImplementation(async () => {
    await readyNotificationsList();
    await waitFor(() => expect(screen.queryByRole("status")).not.toBeInTheDocument());
    const list = screen.getByRole("list", { name: "通知" });
    const scrollRoot = list.parentElement!;
    await expect(screen.getByText("本文 scroll-0")).toBeVisible();
    scrollRoot.scrollTop = scrollRoot.scrollHeight;
    const spinner = await screen.findByRole("status");
    await expect(spinner).toBeVisible();
    await expect(list).toBeVisible();
    await expect(list).not.toHaveAttribute("inert");
    await expect(list).not.toHaveAttribute("aria-hidden");
    await expect(screen.getByText("本文 scroll-0")).toBeInTheDocument();
    await expect(spinner.previousElementSibling).toBe(list);
    await expect(spinner.getBoundingClientRect().bottom).toBeLessThanOrEqual(
      scrollRoot.getBoundingClientRect().bottom,
    );
    scrollRoot.scrollTop = 0;
    await waitFor(() => expect(screen.queryByRole("status")).not.toBeInTheDocument());
    await expect(screen.getByText("本文 scroll-0")).toBeVisible();
  }),
};

let apubItems = structuredClone(activityPubNotifications);
const apubHandlers = [
  http.get("*/api/activitypub/notifications", () =>
    HttpResponse.json({
      items: apubItems,
      unread_count: apubItems.filter((item) => !item.is_read).length,
    }),
  ),
  http.post("*/api/activitypub/follows/respond", async ({ request }) => {
    const body = (await request.json()) as { actor: string; activity_id: string; accept: boolean };
    await expect(body.actor).toBe(activityPubNotifications[0].actor_id);
    await expect(body.activity_id).toBe(activityPubNotifications[0].activity_id);
    apubItems = apubItems.map((item) =>
      item.activity_id === body.activity_id ? { ...item, pending: false, is_read: true } : item,
    );
    return HttpResponse.json({ ok: true });
  }),
  http.post("*/api/activitypub/notifications/read", async ({ request }) => {
    const body = (await request.json()) as { id?: string; ids?: string[] };
    apubItems = apubItems.map((item) =>
      (body.ids ? body.ids.includes(item.id) : !body.id || item.id === body.id)
        ? { ...item, is_read: true }
        : item,
    );
    return HttpResponse.json({ ok: true });
  }),
];
export const FollowRequests: Story = {
  decorators: [withUnreadPreview],
  beforeEach: () => {
    apubItems = structuredClone(activityPubNotifications);
  },
  parameters: {
    collections: { ...storyCollections, notifications: [] },
    msw: { handlers: apubHandlers },
    fixtureControls: {
      assertFollowRead: () => expect(apubItems[0].is_read).toBe(true),
      assertFollowRejected: () => expect(apubItems[0].pending).toBe(false),
    },
  },
  play: playForImplementation(async () => {
    const panel = within(await screen.findByRole("region", { name: "通知" }));
    await expect(await panel.findByRole("listitem", { name: /Bob.*フォロー申請/ })).toBeVisible();
    await expect(panel.getByRole("button", { name: "承認" })).toBeVisible();
    await expect(panel.getByRole("button", { name: "拒否" })).toBeVisible();
    await expect(panel.getByRole("listitem", { name: /Carol.*フォローが承認/ })).toBeVisible();
    await expect(panel.getByRole("listitem", { name: /Dave.*リアクション/ })).toBeVisible();
    await expect(panel.queryByText("投稿にリアクションしました")).not.toBeInTheDocument();
    await expect(panel.getByRole("img", { name: "Bob" })).toBeVisible();
    const profileLinks = panel.getAllByRole("link", { name: "Bob" });
    await expect(profileLinks).toHaveLength(2);
    for (const link of profileLinks)
      await expect(link).toHaveAttribute(
        "href",
        `/home/users/${encodeURIComponent(activityPubNotifications[0].actor_id)}`,
      );
  }),
};
export const ApproveFollow: Story = {
  ...FollowRequests,
  play: playForImplementation(async () => {
    const panel = within(await screen.findByRole("region", { name: "通知" }));
    await userEvent.click(await panel.findByRole("button", { name: "承認" }));
    await waitFor(() =>
      expect(panel.queryByRole("button", { name: "承認" })).not.toBeInTheDocument(),
    );
    await expect(apubItems[0].is_read).toBe(true);
    await userEvent.click(panel.getByRole("button", { name: "すべて既読にする" }));
    await waitFor(() =>
      expect(panel.queryByRole("button", { name: "すべて既読にする" })).not.toBeInTheDocument(),
    );
  }),
};
export const RejectFollow: Story = {
  ...FollowRequests,
  play: playForImplementation(async () => {
    const panel = within(await screen.findByRole("region", { name: "通知" }));
    await userEvent.click(await panel.findByRole("button", { name: "拒否" }));
    await waitFor(() =>
      expect(panel.queryByRole("button", { name: "拒否" })).not.toBeInTheDocument(),
    );
    await expect(apubItems[0].pending).toBe(false);
  }),
};
export const FollowDecisionError: Story = {
  ...FollowRequests,
  parameters: {
    ...FollowRequests.parameters,
    msw: {
      handlers: [
        http.post(
          "*/api/activitypub/follows/respond",
          () => new HttpResponse(null, { status: 500 }),
        ),
        ...apubHandlers,
      ],
    },
  },
  play: playForImplementation(async () => {
    const panel = within(await screen.findByRole("region", { name: "通知" }));
    await userEvent.click(await panel.findByRole("button", { name: "承認" }));
    await expect(await panel.findByRole("alert")).toBeVisible();
    await waitFor(() => expect(panel.getByRole("button", { name: "承認" })).toBeEnabled());
  }),
};
export const MobileFollowRequests: Story = {
  ...FollowRequests,
  globals: { viewport: { value: "mobile" } },
};

export const HomeNotificationsError: Story = {
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/activitypub/notifications", () => new HttpResponse(null, { status: 500 })),
      ],
    },
  },
  play: playForImplementation(async () => {
    const panel = within(await screen.findByRole("region", { name: "通知" }));
    await expect(await panel.findByRole("alert")).toBeVisible();
    await expect(panel.getByRole("button", { name: "更新" })).toBeVisible();
  }),
};
const notificationsLive = ws.link("*/api/home/live");
let liveItems: typeof activityPubNotifications = [];
let notificationConnection = false;
export const LiveFollowRequest: Story = {
  beforeEach: () => {
    liveItems = [];
    notificationConnection = false;
  },
  parameters: {
    fixtureControls: {
      publishFollowRequest: async () => {
        await waitFor(() => expect(notificationConnection).toBe(true));
        liveItems = structuredClone(activityPubNotifications);
        notificationsLive.broadcast("refresh");
      },
    },
    msw: {
      handlers: [
        http.get("*/api/activitypub/notifications", () =>
          HttpResponse.json({ items: liveItems, unread_count: liveItems.length }),
        ),
        notificationsLive.addEventListener("connection", ({ client }) => {
          client.addEventListener("message", () => {
            notificationConnection = true;
            client.send("refresh");
          });
        }),
      ],
    },
  },
  play: playForImplementation(async () => {
    await waitFor(() => expect(notificationConnection).toBe(true));
    liveItems = structuredClone(activityPubNotifications);
    notificationsLive.broadcast("refresh");
    const panel = within(await screen.findByRole("region", { name: "通知" }));
    await expect(await panel.findByRole("button", { name: "承認" })).toBeVisible();
  }),
};

let pagedRequestApproved = false;
export const OlderFollowRequests: Story = {
  beforeEach: () => {
    pagedRequestApproved = false;
  },
  parameters: {
    fixtureControls: {
      assertOlderFollowApproved: () => waitFor(() => expect(pagedRequestApproved).toBe(true)),
    },
    msw: {
      handlers: [
        http.get("*/api/activitypub/notifications", ({ request }) => {
          const older = new URL(request.url).searchParams.get("before");
          const item = {
            ...activityPubNotifications[0],
            id: "older-follow-request",
            display_name: "Older peer",
            activity_id: "https://social.example/follows/older",
            pending: !pagedRequestApproved,
          };
          return HttpResponse.json({
            items: older ? [item] : [activityPubNotifications[1]],
            unread_count: 0,
            next_cursor: older ? null : "accepted-peer",
          });
        }),
        http.post("*/api/activitypub/follows/respond", async ({ request }) => {
          const decision = (await request.json()) as { activity_id: string; accept: boolean };
          if (decision.activity_id !== "https://social.example/follows/older" || !decision.accept)
            return new HttpResponse(null, { status: 400 });
          pagedRequestApproved = true;
          return HttpResponse.json({ ok: true });
        }),
      ],
    },
  },
  play: playForImplementation(async () => {
    const panel = within(await screen.findByRole("region", { name: "通知" }));
    await userEvent.click(await panel.findByRole("button", { name: "過去の通知を読む" }));
    await expect(await panel.findByText("Older peer")).toBeVisible();
    await userEvent.click(await panel.findByRole("button", { name: "承認" }));
    await waitFor(() => expect(pagedRequestApproved).toBe(true));
  }),
};

export const ActivityPubMfmReply: Story = {
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/activitypub/notifications", () =>
          HttpResponse.json({
            items: [
              {
                ...activityPubNotifications[0],
                id: "mfm-reply",
                kind: "reply",
                pending: false,
                content: "**装飾付きの返信** :blobcat:",
                content_emojis: [blobcat],
              },
            ],
            unread_count: 1,
          }),
        ),
      ],
    },
  },
  play: playForImplementation(async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect((await canvas.findByText("装飾付きの返信")).tagName).toBe("STRONG");
    const card = within(await canvas.findByRole("listitem", { name: /Bob.*返信/ }));
    await expect(card.getByRole("img", { name: ":blobcat:" })).toHaveAttribute(
      "src",
      expect.stringContaining("/api/activitypub/emoji-images/"),
    );
  }),
};

const mixedLocal = [
  { ...storyNotifications[0]!, created_at: "2026-11-01T12:00:00Z" },
  { ...storyNotifications[2]!, created_at: "2026-11-01T10:00:00Z" },
];
const mixedRemote = [
  {
    ...activityPubNotifications[2]!,
    id: "remote-reaction",
    display_name: "Remote Fox",
    reaction: "🦊",
    created_at: "2026-11-01T11:30:00Z",
  },
  {
    ...activityPubNotifications[2]!,
    id: "remote-reply",
    display_name: "Remote Reply",
    kind: "reply" as const,
    content: "外部から届いた返信",
    created_at: "2026-11-01T11:00:00Z",
  },
  { ...activityPubNotifications[0]!, created_at: "2026-11-01T09:00:00Z" },
];
export const UnifiedTimeline: Story = {
  decorators: [withUnreadPreview],
  parameters: {
    collections: { ...storyCollections, notifications: mixedLocal },
    msw: {
      handlers: [
        http.get("*/api/activitypub/notifications", () =>
          HttpResponse.json({ items: mixedRemote, unread_count: 3 }),
        ),
      ],
    },
  },
  play: playForImplementation(async () => {
    const list = await readyNotificationsList();
    const rows = Array.from(list.children);
    await expect(rows).toHaveLength(5);
    await expect(rows.map((row) => row.getAttribute("aria-label"))).toEqual([
      expect.stringMatching(/Bob.*メンション/),
      expect.stringMatching(/Remote Fox.*リアクション/),
      expect.stringMatching(/Remote Reply.*返信/),
      expect.stringMatching(/Bob.*リアクション/),
      expect.stringMatching(/Bob.*フォロー申請/),
    ]);
    await expect(rows[1].querySelector("[data-notification-reaction]")).toHaveTextContent("🦊");
    await expect(rows[3].querySelector("[data-notification-reaction]")).not.toBeNull();
    await expect(
      within(list).queryByText("さんが投稿にリアクションしました"),
    ).not.toBeInTheDocument();
    await expect(screen.queryByRole("heading", { name: "ホームの通知" })).not.toBeInTheDocument();
  }),
};
export const UnifiedTimelineMobile: Story = {
  ...UnifiedTimeline,
  globals: { viewport: { value: "mobile" } },
};

let automaticallyRead: string[] = [];
let autoItems = structuredClone(mixedRemote);
export const UnifiedAutoRead: Story = {
  decorators: [withForegroundPreview],
  beforeEach: () => {
    automaticallyRead = [];
    autoItems = structuredClone(mixedRemote);
  },
  parameters: {
    collections: { ...storyCollections, notifications: mixedLocal },
    fixtureControls: {
      assertFederatedAutoRead: () =>
        waitFor(() => expect(automaticallyRead.length).toBeGreaterThan(0)),
    },
    msw: {
      handlers: [
        http.patch("*/api/notifications/:id/read", () =>
          HttpResponse.json({ data: { is_read: true }, txid: null }),
        ),
        http.get("*/api/activitypub/notifications", () =>
          HttpResponse.json({
            items: autoItems,
            unread_count: autoItems.filter((row) => !row.is_read).length,
          }),
        ),
        http.post("*/api/activitypub/notifications/read", async ({ request }) => {
          const body = (await request.json()) as { ids?: string[] };
          if (!body.ids?.length) return new HttpResponse(null, { status: 400 });
          automaticallyRead.push(...body.ids);
          autoItems = autoItems.map((item) =>
            body.ids!.includes(item.id) ? { ...item, is_read: true } : item,
          );
          return HttpResponse.json({ ok: true });
        }),
      ],
    },
  },
  play: playForImplementation(async () => {
    await readyNotificationsList();
    await waitFor(() => expect(automaticallyRead).toContain("remote-reaction"));
    await waitFor(() => expect(automaticallyRead).toContain("remote-reply"));
    await expect(autoItems.find((row) => row.id === "remote-reaction")?.is_read).toBe(true);
  }),
};

export const CustomReactionBadge: Story = {
  decorators: [withUnreadPreview],
  parameters: {
    collections: { ...storyCollections, notifications: [] },
    msw: {
      handlers: [
        http.get("*/api/activitypub/notifications", () =>
          HttpResponse.json({
            items: [
              {
                ...mixedRemote[0],
                display_name: "Remote Blob",
                reaction: blobcat.name,
                emoji: blobcat,
              },
            ],
            unread_count: 1,
          }),
        ),
      ],
    },
  },
  play: playForImplementation(async () => {
    const list = await readyNotificationsList();
    const badge = list.querySelector("[data-notification-reaction]")!;
    await expect(badge).toHaveAttribute("aria-label", blobcat.name);
    await expect(badge.querySelector("img")).toHaveAttribute(
      "src",
      expect.stringContaining("/api/activitypub/emoji-images/"),
    );
    await expect(badge.getBoundingClientRect().width).toBeLessThanOrEqual(20);
  }),
};
