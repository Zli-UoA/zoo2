import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vite-plus/test";
import { page } from "vite-plus/test/browser/context";
import { LocaleProvider } from "../../i18n/LocaleProvider";
import { ApiError } from "../../../lib/api/client";
import { AppConsole } from "./AppConsole";
import type { AppDetail, ConsoleSection } from "./model";
import "../../../styles.css";

const api = vi.hoisted(() => ({
  get: vi.fn(),
  request: vi.fn(),
  post: vi.fn(),
  delete: vi.fn(),
  navigate: vi.fn(),
  admin: false,
  botName: "echo_bot",
}));
vi.mock("../../../lib/api/client", async (original) => ({
  ...(await original<typeof import("../../../lib/api/client")>()),
  createConfiguredApiClient: () => api,
}));
vi.mock("../../../lib/electric/collections-context", async (original) => ({
  ...(await original<typeof import("../../../lib/electric/collections-context")>()),
  useCollections: () => ({ config: { apiBaseUrl: "/" } }),
}));
vi.mock("../../../hooks/useCurrentUser", () => ({
  useCurrentUser: () => ({ currentUserId: "me", isAdmin: api.admin }),
}));
vi.mock("../../../hooks/useRealtime", async (original) => ({
  ...(await original<typeof import("../../../hooks/useRealtime")>()),
  useUsers: () => ({
    data: [
      { id: "me", display_name: "Me", user_name: "me" },
      { id: "bot", display_name: "Echo", user_name: api.botName },
    ],
  }),
  useActiveChannels: () => ({
    data: [
      {
        id: "mine",
        channel_name: "deploys",
        created_user_id: "me",
        is_dm: false,
        archive_at: null,
      },
      {
        id: "theirs",
        channel_name: "alerts",
        created_user_id: "other",
        is_dm: false,
        archive_at: null,
      },
    ],
  }),
}));
vi.mock("@tanstack/react-router", async (original) => ({
  ...(await original<typeof import("@tanstack/react-router")>()),
  useNavigate: () => api.navigate,
  Link: ({ children }: { children: React.ReactNode }) => <a href="/">{children}</a>,
}));

const detail: AppDetail = {
  id: "echo",
  name: "Echo",
  description: "Echo app",
  icon_url: null,
  bot_user_id: "bot",
  owner_id: "me",
  status: "installed",
  revision: 2,
  updated_at: "2026-11-07T00:00:00.123",
  manifest: {
    scopes: ["messages:write"],
    events: [],
    commands: ["deploy"],
    redirect_uris: ["https://example.com/callback"],
    transport: "http",
    event_url: "https://example.com/draft",
  },
  installation: {
    revision: 1,
    active: true,
    transport: "http",
    event_url: "https://example.com/approved",
    endpoint_verified_at: null,
  },
};
const catalog = {
  native_events: [
    { name: "message.created", scope: "messages:read", audience: "channel" },
    { name: "app.mentioned", scope: "messages:read", audience: "channel" },
  ],
};
let current: AppDetail;
beforeEach(async () => {
  await page.viewport(1280, 800);
  current = structuredClone(detail);
  api.admin = false;
  api.botName = "echo_bot";
  api.navigate.mockReset();
  api.get.mockReset().mockImplementation((path: string) => {
    if (path === "/api/apps/echo") return Promise.resolve(current);
    if (path === "/api/apps/events") return Promise.resolve(catalog);
    if (path === "/api/apps/echo/webhooks")
      return Promise.resolve({
        webhooks: [
          {
            id: "hook",
            name: "Deploys",
            path: "/api/webhooks/hook?token=secret",
            channel_id: "mine",
            channel_name: "deploys",
            created_at: "2026-11-07",
          },
        ],
        hidden_count: 2,
      });
    return Promise.resolve([]);
  });
  api.request.mockReset().mockResolvedValue({});
  api.post.mockReset().mockResolvedValue({ data: {} });
  api.delete.mockReset().mockResolvedValue({ data: {} });
});
function open(section: ConsoleSection) {
  render(
    <LocaleProvider>
      <AppConsole appId="echo" section={section} />
    </LocaleProvider>,
  );
}
function patchBody() {
  const call = api.request.mock.calls.find(([, options]) => options?.method === "PATCH");
  return call?.[1]?.body as Record<string, any>;
}

describe("App console", () => {
  it("saves one section without dropping settings from the others", async () => {
    open("events");
    await userEvent.click(await screen.findByRole("checkbox", { name: /message\.created/ }));
    const bar = screen.getByRole("region", { name: "未保存の変更" });
    expect(bar.textContent).toContain("権限");
    expect(bar.textContent).toContain("イベント購読");
    expect(bar.textContent).toContain("再承認まで App が停止します");
    await userEvent.click(within(bar).getByRole("button", { name: "保存" }));
    await waitFor(() => expect(patchBody()).toBeDefined());
    expect(patchBody()).toMatchObject({
      name: "Echo",
      expected_updated_at: detail.updated_at,
      bot_username: null,
      manifest: {
        events: ["message.created"],
        scopes: ["messages:write", "messages:read"],
        commands: ["deploy"],
        redirect_uris: ["https://example.com/callback"],
        event_url: "https://example.com/draft",
      },
    });
  });

  it("stops on a conflicting save and reloads without overwriting", async () => {
    api.request.mockImplementation((_path: string, options?: { method?: string }) =>
      options?.method === "PATCH"
        ? Promise.reject(new ApiError(409, "conflict"))
        : Promise.resolve({}),
    );
    open("basic");
    const name = await screen.findByRole("textbox", { name: "App名" });
    await userEvent.type(name, " 2");
    expect(screen.getByRole("region", { name: "未保存の変更" }).textContent).not.toContain(
      "再承認",
    );
    await userEvent.click(screen.getByRole("button", { name: "保存" }));
    expect((await screen.findByRole("alert")).textContent).toContain(
      "ほかの画面で設定が更新されました",
    );
    expect(screen.getByRole("button", { name: "保存" })).toBeDisabled();
    current = { ...current, name: "Renamed elsewhere", updated_at: "2026-11-07T00:00:01.000" };
    await userEvent.click(screen.getByRole("button", { name: "再読み込み" }));
    await waitFor(() =>
      expect(screen.getByRole("textbox", { name: "App名" })).toHaveValue("Renamed elsewhere"),
    );
    expect(screen.queryByRole("region", { name: "未保存の変更" })).toBeNull();
  });

  it("keeps the conflict baseline when an action reloads the app during edits", async () => {
    open("events");
    await userEvent.click(await screen.findByRole("checkbox", { name: /message\.created/ }));
    // Someone else changes the description before this screen reloads after verification.
    current = {
      ...current,
      description: "Changed elsewhere",
      updated_at: "2026-11-07T00:00:05.000",
    };
    await userEvent.click(screen.getByRole("button", { name: "承認済み URL を検証" }));
    expect((await screen.findByRole("alert")).textContent).toContain(
      "ほかの画面で設定が更新されました",
    );
    const bar = screen.getByRole("region", { name: "未保存の変更" });
    expect(within(bar).getByRole("button", { name: "保存" })).toBeDisabled();
    expect(patchBody()).toBeUndefined();
  });

  it("reports a bot username changed elsewhere instead of reverting it", async () => {
    open("events");
    await userEvent.click(await screen.findByRole("checkbox", { name: /message\.created/ }));
    api.botName = "echo2";
    current = { ...current, updated_at: "2026-11-07T00:00:05.000" };
    await userEvent.click(screen.getByRole("button", { name: "承認済み URL を検証" }));
    expect((await screen.findByRole("alert")).textContent).toContain(
      "ほかの画面で設定が更新されました",
    );
  });

  it("does not report its own save as a conflict after a failed reload", async () => {
    let failNext = false;
    api.get.mockImplementation((path: string) => {
      if (path === "/api/apps/events") return Promise.resolve(catalog);
      if (path !== "/api/apps/echo") return Promise.resolve([]);
      if (failNext) {
        failNext = false;
        return Promise.reject(new Error("offline"));
      }
      return Promise.resolve(current);
    });
    api.request.mockImplementation(
      (_path: string, options?: { method?: string; body?: Record<string, any> }) => {
        if (options?.method === "PATCH") {
          current = {
            ...current,
            manifest: options.body!.manifest,
            updated_at: "2026-11-07T00:00:09.000",
          };
        }
        return Promise.resolve({});
      },
    );
    open("events");
    await userEvent.click(await screen.findByRole("checkbox", { name: /message\.created/ }));
    failNext = true;
    await userEvent.click(screen.getByRole("button", { name: "承認済み URL を検証" }));
    await waitFor(() =>
      expect(api.get.mock.calls.filter(([path]) => path === "/api/apps/echo")).toHaveLength(2),
    );
    const bar = screen.getByRole("region", { name: "未保存の変更" });
    await userEvent.click(within(bar).getByRole("button", { name: "保存" }));
    await waitFor(() => expect(screen.queryByRole("region", { name: "未保存の変更" })).toBeNull());
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("keeps edits and takes the new version when a reload finds no other changes", async () => {
    open("events");
    await userEvent.click(await screen.findByRole("checkbox", { name: /message\.created/ }));
    current = { ...current, updated_at: "2026-11-07T00:00:05.000" };
    await userEvent.click(screen.getByRole("button", { name: "承認済み URL を検証" }));
    await waitFor(() =>
      expect(api.get.mock.calls.filter(([path]) => path === "/api/apps/echo")).toHaveLength(2),
    );
    const bar = screen.getByRole("region", { name: "未保存の変更" });
    expect(screen.queryByRole("alert")).toBeNull();
    await userEvent.click(within(bar).getByRole("button", { name: "保存" }));
    await waitFor(() => expect(patchBody()).toBeDefined());
    expect(patchBody()).toMatchObject({
      expected_updated_at: "2026-11-07T00:00:05.000",
      manifest: { events: ["message.created"] },
    });
  });

  it("verifies the approved URL and says the edited one needs re-approval", async () => {
    open("events");
    const verification = (
      await screen.findByRole("heading", { name: "承認済み URL の検証" })
    ).closest("section")!;
    expect(verification.textContent).toContain("https://example.com/approved");
    expect(verification.textContent).toContain(
      "編集中の URL（https://example.com/draft）は再承認後に検証できます。",
    );
    await userEvent.click(
      within(verification).getByRole("button", { name: "承認済み URL を検証" }),
    );
    await waitFor(() =>
      expect(api.request).toHaveBeenCalledWith("/api/apps/echo/actions/verify-endpoint", {
        method: "POST",
      }),
    );
  });

  it("does not offer verification before the first approval", async () => {
    current = { ...current, status: "draft", installation: null };
    open("events");
    const verification = (
      await screen.findByRole("heading", { name: "承認済み URL の検証" })
    ).closest("section")!;
    expect(verification.textContent).toContain("承認後に検証できます。");
    expect(within(verification).queryByRole("button")).toBeNull();
  });

  it("shows the signing secret on request and the bot token only once", async () => {
    api.request.mockImplementation((path: string) =>
      Promise.resolve(
        path.endsWith("/connection")
          ? { signing_secret: "zoo_sign_secret" }
          : path.endsWith("/token")
            ? { access_token: "zoo_at_x" }
            : {},
      ),
    );
    open("credentials");
    expect(await screen.findByText(/表示するたびに監査履歴に記録されます/)).toBeVisible();
    await userEvent.click(screen.getByRole("button", { name: "表示" }));
    await waitFor(() => expect(screen.getByDisplayValue("zoo_sign_secret")).toBeInTheDocument());
    await userEvent.click(screen.getByRole("button", { name: "トークンを発行" }));
    expect(await screen.findByText(/この値は今だけ表示されます/)).toBeVisible();
  });

  it("rejects malformed manifest JSON without saving", async () => {
    open("manifest");
    const manifest = await screen.findByRole("textbox", { name: "App設定（JSON）" });
    await userEvent.clear(manifest);
    await userEvent.type(manifest, "invalid JSON");
    await userEvent.click(screen.getByRole("button", { name: "反映" }));
    expect(screen.getByRole("alert").textContent).toContain(
      "Manifestに有効なJSONを入力してください。",
    );
    expect(screen.queryByRole("region", { name: "未保存の変更" })).toBeNull();
    expect(patchBody()).toBeUndefined();
  });

  it("limits webhooks to managed channels and counts the rest", async () => {
    current = { ...current, manifest: { notification_only: true, scopes: [], events: [] } };
    open("webhooks");
    const hooks = (await screen.findByRole("heading", { name: "この App の Webhook" })).closest(
      "section",
    )!;
    expect(await within(hooks).findByText("#deploys")).toBeVisible();
    expect(screen.getByText(/ほかに 2 件あります/)).toBeVisible();
    const channel = screen.getByRole("combobox", { name: "投稿先チャンネル" });
    expect(within(channel).queryByRole("option", { name: "#alerts" })).toBeNull();
    await userEvent.selectOptions(channel, "mine");
    await userEvent.click(screen.getByRole("button", { name: "作成" }));
    await waitFor(() =>
      expect(api.post).toHaveBeenCalledWith("/api/channels/mine/webhooks", {
        name: "Webhook",
        app_id: "echo",
      }),
    );
  });

  it("reloads the app after a webhook is removed", async () => {
    current = { ...current, manifest: { notification_only: true, scopes: [], events: [] } };
    open("webhooks");
    const hooks = (await screen.findByRole("heading", { name: "この App の Webhook" })).closest(
      "section",
    )!;
    await within(hooks).findByText("#deploys");
    // The server disables a notification-only app when its last webhook is removed.
    current = { ...current, status: "disabled" };
    await userEvent.click(within(hooks).getByRole("button", { name: "削除" }));
    await userEvent.click(within(hooks).getByRole("button", { name: "実行" }));
    await waitFor(() =>
      expect(api.delete).toHaveBeenCalledWith("/api/channels/mine/webhooks/hook"),
    );
    await waitFor(() => expect(screen.queryByRole("button", { name: "作成" })).toBeNull());
    expect(screen.getByRole("heading", { level: 1 }).parentElement!.textContent).toContain("無効");
  });

  it("names the manifest field that has the wrong type", async () => {
    open("manifest");
    const manifest = await screen.findByRole("textbox", { name: "App設定（JSON）" });
    // Replace the whole value at once; clearing a long controlled textarea is unreliable here.
    fireEvent.change(manifest, { target: { value: '{"scopes":42}' } });
    await userEvent.click(screen.getByRole("button", { name: "反映" }));
    expect(screen.getByRole("alert").textContent).toContain("「scopes」");
    expect(screen.queryByRole("region", { name: "未保存の変更" })).toBeNull();
  });

  it("hides incoming webhooks for regular apps", async () => {
    open("basic");
    await screen.findByRole("textbox", { name: "App名" });
    const nav = screen.getByRole("navigation", { name: "設定項目" });
    expect(within(nav).queryByRole("button", { name: "Incoming Webhook" })).toBeNull();
  });

  it("switches sections with a select on narrow screens", async () => {
    await page.viewport(390, 800);
    open("basic");
    const sections = await screen.findByRole("combobox", { name: "設定項目" });
    expect(screen.queryByRole("navigation", { name: "設定項目" })).toBeNull();
    await userEvent.selectOptions(sections, "events");
    expect(api.navigate).toHaveBeenCalledWith({
      to: "/apps/$appId",
      params: { appId: "echo" },
      search: { section: "events" },
      replace: true,
    });
  });

  it("confirms deletion and returns to the app list", async () => {
    open("install");
    await userEvent.click(await screen.findByRole("button", { name: "Appを削除" }));
    const dialog = await screen.findByRole("alertdialog");
    expect(dialog.textContent).toContain("「Echo」を削除しますか？");
    await userEvent.click(within(dialog).getByRole("button", { name: "Appを削除" }));
    await waitFor(() =>
      expect(api.request).toHaveBeenCalledWith("/api/apps/echo/actions/delete", { method: "POST" }),
    );
    await waitFor(() => expect(api.navigate).toHaveBeenCalledWith({ to: "/apps" }));
  });
});
