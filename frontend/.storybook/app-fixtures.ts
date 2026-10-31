import { http, HttpResponse } from "msw";
export const storyApps = [
  {
    id: "echo",
    name: "Echo Bot",
    description: "チャンネルの会話をサポートするAppです。",
    owner_id: "user-alice",
    status: "installed",
    revision: 1,
    manifest: {
      scopes: ["messages:read", "messages:write"],
      transport: "websocket",
      dm_enabled: true,
    },
  },
  {
    id: "notes",
    name: "Meeting Notes",
    description: "会議のメモを共有します。",
    owner_id: "user-bob",
    status: "installed",
    revision: 1,
    manifest: { scopes: ["messages:write"] },
  },
  {
    id: "release",
    name: "Release Tracker",
    description: "リリース情報を共有するAppです。",
    owner_id: "user-alice",
    status: "draft",
    revision: 1,
    manifest: {},
  },
];
export const appHandlers = [
  http.get("*/api/apps", () => HttpResponse.json(storyApps)),
  http.get("*/api/apps/:id/surfaces", () => HttpResponse.json([])),
];
