import { http, HttpResponse } from "msw";

const transparentPng = Uint8Array.from(
  atob(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==",
  ),
  (char) => char.charCodeAt(0),
);

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
    return HttpResponse.json([
      {
        id: "webhook-1",
        name: "GitHub",
        path: `/api/webhooks/${channelId}/github`,
        generic_path: `/api/webhooks/${channelId}/generic`,
        secret: "storybook-example-secret",
        created_at: "2026-01-02T09:00:00.000Z",
      },
    ]);
  }),
  http.get("*/api/push/config", () => HttpResponse.json({ publicKey: null })),
  http.get("https://www.gstatic.com/**", () =>
    HttpResponse.arrayBuffer(transparentPng.buffer as ArrayBuffer, {
      headers: { "Content-Type": "image/png" },
    }),
  ),
];
