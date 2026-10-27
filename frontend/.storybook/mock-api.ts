import { http, HttpResponse } from "msw";

// Emoji Kitchen のプレビューとして使う、猫耳付きのスマイリー。
const kitchenPreview = `<svg xmlns="http://www.w3.org/2000/svg" width="72" height="72" viewBox="0 0 72 72">
  <defs>
    <linearGradient id="face" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="#fde68a"/>
      <stop offset="1" stop-color="#f59e0b"/>
    </linearGradient>
  </defs>
  <rect width="72" height="72" rx="18" fill="url(#face)"/>
  <path d="M14 25 20 8l15 12Z" fill="#fbbf24" stroke="#b45309" stroke-width="2" stroke-linejoin="round"/>
  <path d="M58 25 52 8 37 20Z" fill="#fbbf24" stroke="#b45309" stroke-width="2" stroke-linejoin="round"/>
  <circle cx="27" cy="37" r="4" fill="#3f2a14"/>
  <circle cx="45" cy="37" r="4" fill="#3f2a14"/>
  <circle cx="20" cy="47" r="4" fill="#fb7185" opacity=".65"/>
  <circle cx="52" cy="47" r="4" fill="#fb7185" opacity=".65"/>
  <path d="M28 50c4 5 12 5 16 0" fill="none" stroke="#3f2a14" stroke-width="3" stroke-linecap="round"/>
</svg>`;

const lastReadAt = "2026-01-02 10:00:00.000";

interface NotificationContextFixture {
  message: { id: string; user_id: string; content: string };
  reactions: { user_id: string; emoji: string }[];
}

const notificationContexts: Record<string, NotificationContextFixture> = {
  "notification-mention": {
    message: {
      id: "message-hello",
      user_id: "user-alice",
      content:
        "@channel おはようございます。今日のリリースは 18:00 です。詳細は https://example.com/docs を参照してください。",
    },
    reactions: [
      { user_id: "user-alice", emoji: "👍" },
      { user_id: "user-bob", emoji: "👍" },
      { user_id: "user-carol", emoji: "🎉" },
    ],
  },
  "notification-reply": {
    message: {
      id: "message-thread-reply",
      user_id: "user-bob",
      content: "確認しました。2 点だけコメントしました。",
    },
    reactions: [],
  },
  "notification-reaction": {
    message: {
      id: "message-thread-root",
      user_id: "user-carol",
      content: "リリースノートの下書きを共有しました。レビューをお願いします。",
    },
    reactions: [
      { user_id: "user-bob", emoji: "🎉" },
      { user_id: "user-alice", emoji: "👍" },
    ],
  },
  "notification-reaction-read": {
    message: {
      id: "message-bob-reply",
      user_id: "user-bob",
      content: "了解です！デプロイ後に確認します 👍",
    },
    reactions: [{ user_id: "user-alice", emoji: "❤️" }],
  },
};

export const storybookApiHandlers = [
  http.get("*/api/users/me", () =>
    HttpResponse.json({
      id: "user-alice",
      user_name: "alice",
      display_name: "Alice",
      is_admin: false,
    }),
  ),
  http.get(/\/api\/channels\/[^/]+\/read-state$/, ({ request }) => {
    const channelId = new URL(request.url).pathname.split("/")[3] ?? "channel-general";
    return HttpResponse.json({
      channel_id: channelId,
      last_read_at: lastReadAt,
      has_unread: false,
    });
  }),
  http.post(/\/api\/channels\/[^/]+\/read-state$/, ({ request }) => {
    const channelId = new URL(request.url).pathname.split("/")[3] ?? "channel-general";
    return HttpResponse.json({
      channel_id: channelId,
      last_read_at: lastReadAt,
      has_unread: false,
    });
  }),
  http.get(/\/api\/channels\/[^/]+\/typing$/, () => HttpResponse.json([])),
  http.post(/\/api\/channels\/[^/]+\/typing$/, () => new HttpResponse(null, { status: 204 })),
  http.get("*/api/bubble-chat", () => HttpResponse.json([])),
  http.post("*/api/bubble-chat", () => HttpResponse.json({ ok: true })),
  http.get("*/api/channel-read-states", () => HttpResponse.json([])),
  http.get("*/api/users/admins", () => HttpResponse.json({ user_ids: ["user-admin"] })),
  http.get("*/api/notifications/:notificationId/context", ({ params }) => {
    const notificationId = String(params.notificationId);
    return HttpResponse.json(
      notificationContexts[notificationId] ?? {
        message: null,
        reactions: [],
      },
    );
  }),
  http.get("*/api/channels/:channelId/webhooks", ({ params }) => {
    const channelId = String(params.channelId);
    return HttpResponse.json({
      data: [
        {
          id: "webhook-1",
          name: "GitHub",
          path: `/api/webhooks/${channelId}/github`,
          generic_path: `/api/webhooks/${channelId}/generic`,
          secret: "storybook-example-secret",
          created_at: "2026-01-02T09:00:00.000Z",
        },
      ],
      txid: null,
    });
  }),
  http.get("*/api/push/config", () => HttpResponse.json({ publicKey: null })),
  http.get("https://www.gstatic.com/**", () =>
    HttpResponse.text(kitchenPreview, { headers: { "Content-Type": "image/svg+xml" } }),
  ),
];
