import { createSecureServer } from "node:http2";
import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
const tlsDir = mkdtempSync(join(tmpdir(), "zoo-compose-tls-"));
execFileSync(
  "openssl",
  [
    "req",
    "-x509",
    "-newkey",
    "rsa:2048",
    "-nodes",
    "-days",
    "1",
    "-subj",
    "/CN=localhost",
    "-addext",
    "subjectAltName=DNS:localhost,IP:127.0.0.1",
    "-keyout",
    join(tlsDir, "key.pem"),
    "-out",
    join(tlsDir, "cert.pem"),
  ],
  { stdio: "ignore" },
);
const tls = {
  key: readFileSync(join(tlsDir, "key.pem")),
  cert: readFileSync(join(tlsDir, "cert.pem")),
  allowHTTP1: true,
};
rmSync(tlsDir, { recursive: true, force: true });
let tx = 1;
let state;
const streams = new Set();
const now = "2026-10-30 09:00:00.000";
const person = (id, name) => ({
  id,
  user_name: name.toLowerCase(),
  display_name: name,
  is_bot: false,
  app_id: null,
  avatar_url: null,
  bot_mentionable: false,
  created_at: now,
  updated_at: null,
});
const channel = (id, name, extra = {}) => ({
  id,
  channel_name: name,
  description: "共通の画面と操作",
  created_user_id: "00000000-0000-0000-0000-000000000001",
  is_private: false,
  is_dm: false,
  created_at: now,
  updated_at: null,
  archive_at: null,
  deleted_at: null,
  ...extra,
});
const message = (id, content, extra = {}) => ({
  id,
  channel_id: "c1",
  user_id: "00000000-0000-0000-0000-000000000001",
  content,
  parent_id: null,
  created_at: now,
  updated_at: null,
  deleted_at: null,
  card: null,
  app_blocks: null,
  app_attachments: null,
  app_unfurls: null,
  app_origin_id: null,
  ...extra,
});
function reset() {
  tx = 1;
  state = {
    preferences: { search_engine: "google" },
    admins: ["00000000-0000-0000-0000-000000000001"],
    protectedAdmins: ["00000000-0000-0000-0000-000000000001"],
    commands: [{ name: "echo", app_name: "Meeting Notes", app_id: "notes" }],
    commandCalls: [],
    bubbleWrites: [],
    bubbles: {},
    previewRequests: [],
    mediaRequests: [],
    users: [
      person("00000000-0000-0000-0000-000000000001", "Alice"),
      person("00000000-0000-0000-0000-000000000002", "Bob"),
    ],
    channels: [channel("c1", "general"), channel("c2", "private", { is_private: true })],
    channel_users: [
      { user_id: "00000000-0000-0000-0000-000000000001", channel_id: "c1", joined_at: now },
      { user_id: "00000000-0000-0000-0000-000000000002", channel_id: "c1", joined_at: now },
      { user_id: "00000000-0000-0000-0000-000000000001", channel_id: "c2", joined_at: now },
    ],
    messages: [
      message("m1", "こんにちは Parity **太字** @bob\n`code`"),
      message("m2", "返信の本文", {
        parent_id: "m1",
        user_id: "00000000-0000-0000-0000-000000000002",
      }),
    ],
    message_reactions: [],
    notifications: [
      {
        id: "n1",
        user_id: "00000000-0000-0000-0000-000000000001",
        actor_id: "00000000-0000-0000-0000-000000000002",
        channel_id: "c1",
        message_id: "m1",
        notification_type: "mention",
        is_read: false,
        created_at: now,
      },
    ],
    apps: [],
    hooks: [],
  };
}
reset();
const resourceRows = (name, channelId) => {
  const rows = state[name] ?? [];
  if (name === "messages") return rows.filter((r) => r.channel_id === channelId);
  if (name === "message_reactions")
    return rows.filter((r) =>
      state.messages.some((m) => m.id === r.message_id && m.channel_id === channelId),
    );
  return rows;
};
const key = (r) => r.id ?? `${r.user_id}:${r.channel_id}`;
const control = () => ({ headers: { control: "up-to-date", global_last_seen_lsn: String(tx) } });
const packet = (name, row, operation = "insert") => ({
  key: key(row),
  value: Object.fromEntries(
    Object.entries(row).map(([k, v]) => [
      k,
      v === null ? null : typeof v === "object" ? JSON.stringify(v) : String(v),
    ]),
  ),
  headers: {
    operation,
    relation: ["public", name === "channel_users" ? "channel_user" : name],
    txids: [tx],
  },
});
function emit(name, row, operation = "insert") {
  for (const stream of streams)
    if (
      stream.name === name &&
      (!stream.channelId || row.channel_id === stream.channelId || name === "message_reactions")
    ) {
      stream.res.write(`data: ${JSON.stringify(packet(name, row, operation))}\n\n`);
      stream.res.write(`data: ${JSON.stringify(control())}\n\n`);
    }
}
const appResponse = (res, data, replicated = true) =>
  res.end(JSON.stringify(replicated ? { data, txid: String(tx) } : data));
const server = createSecureServer(tls, async (req, res) => {
  const url = new URL(req.url, "http://localhost");
  res.setHeader("Access-Control-Allow-Origin", "*");
  res.setHeader("Access-Control-Allow-Headers", "Authorization,Content-Type,X-Dev-User");
  res.setHeader("Access-Control-Allow-Methods", "GET,POST,PATCH,DELETE,OPTIONS");
  res.setHeader(
    "Access-Control-Expose-Headers",
    "electric-handle,electric-offset,electric-cursor,electric-schema",
  );
  if (req.method === "OPTIONS") {
    res.writeHead(204).end();
    return;
  }
  if (url.pathname === "/__reset") {
    reset();
    res.end("ok");
    return;
  }
  if (url.pathname === "/health") {
    res.end("ok");
    return;
  }
  res.setHeader("Content-Type", "application/json");
  const shape = url.pathname.match(/^\/api\/shapes\/(\w+)$/);
  if (shape) {
    const name = shape[1],
      channelId = url.searchParams.get("channel_id"),
      rows = resourceRows(name, channelId);
    res.setHeader("electric-handle", `h-${name}-${channelId ?? ""}`);
    res.setHeader("electric-offset", `${tx}_0`);
    res.setHeader("electric-cursor", String(tx));
    const sample =
      rows[0] ??
      (name === "messages" ? message("", "") : name === "channels" ? channel("", "") : { id: "" });
    res.setHeader(
      "electric-schema",
      JSON.stringify(Object.fromEntries(Object.keys(sample).map((k) => [k, { type: "text" }]))),
    );
    if (url.searchParams.get("live_sse") === "true") {
      res.setHeader("Content-Type", "text/event-stream");
      res.setHeader("Cache-Control", "no-cache");
      res.flushHeaders();
      res.write(`data: ${JSON.stringify(control())}\n\n`);
      const stream = { name, channelId, res };
      streams.add(stream);
      res.on("close", () => streams.delete(stream));
      return;
    }
    res.end(JSON.stringify([...rows.map((row) => packet(name, row)), control()]));
    return;
  }
  let body = {};
  if (req.method !== "GET") {
    let raw = "";
    for await (const part of req) raw += part;
    if (raw) body = JSON.parse(raw);
  }
  const path = url.pathname;
  if (path === "/__state") {
    appResponse(res, state, false);
    return;
  }
  if (path === "/__seed") {
    for (const row of body.messages ?? []) {
      state.messages.push(message(row.id, row.content, row));
      emit("messages", state.messages.at(-1));
    }
    appResponse(res, {}, false);
    return;
  }
  if (path.endsWith("/__summaly")) {
    state.previewRequests.push({
      url: url.searchParams.get("url"),
      authorization: req.headers.authorization,
      devUser: req.headers["x-dev-user"],
      cookie: req.headers.cookie,
      referer: req.headers.referer,
    });
    if (url.searchParams.get("url")?.includes("delayed"))
      await new Promise((resolve) => setTimeout(resolve, 800));
    appResponse(
      res,
      {
        title: "Parity URL card",
        description: "遅れて展開しても読んでいる投稿を維持します。".repeat(30),
        sitename: "Parity",
        thumbnail: "https://image.example/wide.png",
      },
      false,
    );
    return;
  }
  if (path.endsWith("/__media")) {
    state.mediaRequests.push({
      authorization: req.headers.authorization,
      devUser: req.headers["x-dev-user"],
      cookie: req.headers.cookie,
      referer: req.headers.referer,
    });
    res.setHeader("Content-Type", "image/png");
    res.end(readFileSync(new URL("../public/icons/icon-192.png", import.meta.url)));
    return;
  }
  if (path === "/api/users/me/preferences") {
    if (req.method === "PATCH") Object.assign(state.preferences, body);
    appResponse(res, state.preferences, false);
    return;
  }
  const role = path.match(/^\/api\/users\/([^/]+)\/admin$/);
  if (role && req.method === "PATCH") {
    const id = role[1];
    if (
      !body.is_admin &&
      (state.protectedAdmins.includes(id) ||
        (state.admins.includes(id) && state.admins.length <= 1))
    ) {
      res.writeHead(409).end("{}");
      return;
    }
    state.admins = body.is_admin
      ? [...new Set([...state.admins, id])]
      : state.admins.filter((value) => value !== id);
    appResponse(res, {}, false);
    return;
  }
  if (path === "/api/apps/commands") {
    appResponse(res, state.commands, false);
    return;
  }
  if (path === "/api/apps/notes/invoke" || path === "/api/apps/commands/echo") {
    state.commandCalls.push({ path, ...body });
    appResponse(res, {}, false);
    return;
  }
  if (path === "/api/bubble") {
    if (req.method === "GET") {
      appResponse(
        res,
        Object.values(state.bubbles)
          .filter((row) => row.content?.trim())
          .map((row) => ({ ...state.users[0], user_id: state.users[0].id, content: row.content })),
        false,
      );
      return;
    }
    state.bubbleWrites.push(body);
    const old = state.bubbles[body.session_id];
    if (old && body.seq <= old.seq) {
      appResponse(res, { ok: false, stale: true, server_time: Date.now() }, false);
      return;
    }
    state.bubbles[body.session_id] = body;
    appResponse(res, { ok: true, server_time: Date.now() }, false);
    return;
  }
  const notificationContext = path.match(/^\/api\/notifications\/([^/]+)\/context$/);
  if (notificationContext) {
    const notice = state.notifications.find((n) => n.id === notificationContext[1]);
    const message = state.messages.find((m) => m.id === notice?.message_id && !m.deleted_at);
    appResponse(
      res,
      {
        message: message ?? null,
        reactions: state.message_reactions.filter((r) => r.message_id === message?.id),
      },
      false,
    );
    return;
  }
  const appDetail = path.match(/^\/api\/apps\/([^/]+)(?:\/(.+))?$/);
  if (appDetail) {
    const app = state.apps.find((item) => item.id === appDetail[1]);
    if (!app) {
      res.writeHead(404).end("{}");
      return;
    }
    const action = appDetail[2];
    if (!action) {
      if (req.method === "PATCH") Object.assign(app, body);
      appResponse(res, app, false);
      return;
    }
    if (action.startsWith("actions/")) {
      const name = action.slice(8);
      if (name === "submit") app.status = "pending";
      if (name === "install") app.status = "installed";
      if (name === "disable") app.status = "disabled";
      if (name === "enable") app.status = "installed";
      appResponse(res, { ...app, token: "fixture-app-token" }, false);
      return;
    }
    if (action === "connection") {
      if (req.method !== "GET") app.connection = body.connect ? "connected" : "disconnected";
      appResponse(res, { state: app.connection ?? "disconnected" }, false);
      return;
    }
    if (action === "surfaces" || action === "deliveries") {
      appResponse(res, [], false);
      return;
    }
    appResponse(res, {}, false);
    return;
  }
  if (path === "/api/users/me") {
    appResponse(res, { ...state.users[0], is_admin: true }, false);
    return;
  }
  if (path === "/api/users/admins") {
    appResponse(res, { user_ids: state.admins, protected_user_ids: state.protectedAdmins }, false);
    return;
  }
  if (path === "/api/push/config") {
    appResponse(res, { publicKey: null }, false);
    return;
  }
  if (path === "/api/app-surfaces" || path === "/api/apps") {
    if (req.method === "POST") {
      const app = {
        id: "app" + (state.apps.length + 1),
        name: body.name,
        description: body.description,
        icon_url: body.icon_url,
        manifest: body.manifest,
        owner_id: "00000000-0000-0000-0000-000000000001",
        bot_user_id: "bot1",
        revision: 1,
        status: "draft",
      };
      state.apps.push(app);
      appResponse(res, app, false);
    } else appResponse(res, path === "/api/apps" ? state.apps : [], false);
    return;
  }
  if (path === "/api/channel-read-states") {
    appResponse(
      res,
      state.channels.map((c) => ({ channel_id: c.id, last_read_at: null, has_unread: false })),
      false,
    );
    return;
  }
  if (path.includes("/typing")) {
    appResponse(res, req.method === "GET" ? [] : {}, false);
    return;
  }
  if (path.includes("/read-state")) {
    appResponse(res, { last_read_at: now, has_unread: false }, req.method !== "GET");
    return;
  }
  if (path.includes("/view") || path === "/api/presence/heartbeat") {
    appResponse(res, {}, false);
    return;
  }
  if (path === "/api/messages" && req.method === "POST") {
    tx++;
    const row = message(body.id, body.content, { ...body, created_at: new Date().toISOString() });
    state.messages.push(row);
    emit("messages", row);
    appResponse(res, row);
    return;
  }
  const reaction = path.match(/^\/api\/messages\/([^/]+)\/reactions$/);
  if (reaction) {
    tx++;
    if (req.method === "POST") {
      const row = {
        id: body.id,
        message_id: reaction[1],
        user_id: "00000000-0000-0000-0000-000000000001",
        emoji: body.emoji,
        created_at: now,
      };
      state.message_reactions.push(row);
      emit("message_reactions", row);
      appResponse(res, row);
    } else {
      const row = state.message_reactions.find(
        (r) => r.message_id === reaction[1] && r.emoji === url.searchParams.get("emoji"),
      );
      state.message_reactions = state.message_reactions.filter((r) => r !== row);
      if (row) emit("message_reactions", row, "delete");
      appResponse(res, row);
    }
    return;
  }
  const msg = path.match(/^\/api\/messages\/([^/]+)$/);
  if (msg) {
    const row = state.messages.find((r) => r.id === msg[1]);
    if (!row) {
      res.writeHead(404).end("{}");
      return;
    }
    if (req.method === "GET") {
      res.writeHead(405).end("{}");
      return;
    }
    tx++;
    Object.assign(row, req.method === "DELETE" ? { deleted_at: now } : body);
    emit("messages", row, "update");
    appResponse(res, row);
    return;
  }
  if (path === "/api/channels" && req.method === "POST") {
    tx++;
    const row = channel(body.id ?? "c" + (state.channels.length + 1), body.channel_name, body);
    state.channels.push(row);
    const member = {
      user_id: "00000000-0000-0000-0000-000000000001",
      channel_id: row.id,
      joined_at: now,
    };
    state.channel_users.push(member);
    emit("channels", row);
    emit("channel_users", member);
    appResponse(res, row);
    return;
  }
  const chan = path.match(/^\/api\/channels\/([^/]+)(?:\/(.+))?$/);
  if (chan) {
    const row = state.channels.find((c) => c.id === chan[1]);
    const action = chan[2];
    tx++;
    if (action === "join" || action === "users") {
      const member = {
        channel_id: chan[1],
        user_id: body.user_id ?? "00000000-0000-0000-0000-000000000001",
        joined_at: now,
      };
      state.channel_users.push(member);
      emit("channel_users", member);
      appResponse(res, member);
      return;
    }
    if (action === "leave" || action?.startsWith("users/")) {
      const uid = action === "leave" ? "00000000-0000-0000-0000-000000000001" : action.slice(6);
      const member = state.channel_users.find((r) => r.channel_id === chan[1] && r.user_id === uid);
      state.channel_users = state.channel_users.filter((r) => r !== member);
      if (member) emit("channel_users", member, "delete");
      appResponse(res, member);
      return;
    }
    if (action === "archive" || action === "unarchive") {
      row.archive_at = action === "archive" ? now : null;
      emit("channels", row, "update");
      appResponse(res, row);
      return;
    }
    if (action?.startsWith("webhooks")) {
      const parts = action.split("/");
      let hook = state.hooks.find((h) => h.id === parts[1] && h.channel_id === chan[1]);
      if (parts.length === 1 && req.method === "POST") {
        hook = {
          id: "hook" + (state.hooks.length + 1),
          channel_id: chan[1],
          name: body.name,
          path: "/api/webhooks/fixture",
          secret: "first-secret",
          created_at: now,
        };
        state.hooks.push(hook);
      } else if (parts[2] === "rotate-secret" && hook) hook.secret = "rotated-secret";
      else if (parts[2] === "app" && hook) hook.app_id = body.app_id;
      else if (req.method === "DELETE" && hook) state.hooks = state.hooks.filter((h) => h !== hook);
      appResponse(
        res,
        parts.length === 1 && req.method === "GET"
          ? state.hooks.filter((h) => h.channel_id === chan[1])
          : (hook ?? {}),
      );
      return;
    }
    if (row) {
      Object.assign(row, body);
      emit("channels", row, "update");
      appResponse(res, row);
      return;
    }
  }
  if (path === "/api/dms" && req.method === "POST") {
    tx++;
    const row = channel("dm1", "", { is_dm: true, is_private: true });
    state.channels.push(row);
    emit("channels", row);
    for (const user_id of ["00000000-0000-0000-0000-000000000001", ...body.target_user_ids]) {
      const m = { user_id, channel_id: row.id, joined_at: now };
      state.channel_users.push(m);
      emit("channel_users", m);
    }
    appResponse(res, row);
    return;
  }
  if (path === "/api/notifications/read-all" || /^\/api\/notifications\/[^/]+\/read$/.test(path)) {
    tx++;
    for (const n of state.notifications) {
      n.is_read = true;
      emit("notifications", n, "update");
    }
    appResponse(res, state.notifications[0]);
    return;
  }
  res
    .writeHead(404)
    .end(JSON.stringify({ error: `Unhandled fixture route: ${req.method} ${path}` }));
});
server.listen(Number(process.env.COMPOSE_API_PORT ?? 4187), "127.0.0.1");
