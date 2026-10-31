import type { CardData } from "../src/components/chat/messages/MessageCard";
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

export const storyCommands = [
  { app_id: "echo", app_name: "Echo Bot", name: "echo" },
  { app_id: "notes", app_name: "Meeting Notes", name: "echo" },
  { app_id: "github", app_name: "GitHub", name: "github" },
];

export const storyDeploymentCard: CardData = {
  title: "Deployment to storybook-pr-271",
  url: "https://github.com/Zli-UoA/zoo2/deployments",
  description: "",
  source: "Zli-UoA/zoo2",
  context: "2026-10-31 19:56",
  author: "",
  badge: "",
  color: "#2da44e",
  fields: [
    { title: "Status", value: "Completed", short: true },
    {
      title: "Commit",
      value: "3968f64 (fix/github-bot-paused-queue)",
      short: true,
      code: true,
      url: "https://github.com/Zli-UoA/zoo2/commit/3968f64",
    },
    {
      title: "Workflow",
      value: "Storybook Preview #68 / deploy",
      url: "https://github.com/Zli-UoA/zoo2/actions",
    },
  ],
};
