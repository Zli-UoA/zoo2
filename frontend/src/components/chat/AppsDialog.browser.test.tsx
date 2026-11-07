import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vite-plus/test";
import { LocaleProvider } from "../i18n/LocaleProvider";
import { AppsPanel } from "./AppsDialog";
import "../../styles.css";

const api = vi.hoisted(() => ({
  get: vi.fn(),
  request: vi.fn(),
  selectedChannelId: "general" as string | null,
  navigate: vi.fn(),
}));
vi.mock("../../lib/api/client", () => ({ createConfiguredApiClient: () => api }));
vi.mock("../../lib/electric/collections-context", () => ({
  useCollections: () => ({ config: {} }),
}));
vi.mock("../../hooks/useCurrentUser", () => ({
  useCurrentUser: () => ({ currentUserId: "me", isAdmin: true }),
}));
vi.mock("../../hooks/useRealtime", () => ({
  useUsers: () => ({ data: [] }),
  useActiveChannels: () => ({ data: [] }),
}));
vi.mock("../layout/shell-context", () => ({
  useShell: () => ({ selectedChannelId: api.selectedChannelId }),
}));
vi.mock("@tanstack/react-router", () => ({ useNavigate: () => api.navigate }));
const apps = [
  {
    id: "alpha",
    name: "Alpha",
    description: "Installed integration",
    owner_id: "other",
    status: "installed",
    manifest: { scopes: ["messages:write"] },
    revision: 1,
  },
  {
    id: "draft",
    name: "Draft",
    description: "My integration",
    owner_id: "me",
    status: "draft",
    manifest: {},
    revision: 1,
  },
];
beforeEach(() => {
  api.selectedChannelId = "general";
  api.get
    .mockReset()
    .mockImplementation((path: string) => Promise.resolve(path === "/api/apps" ? apps : []));
  api.request.mockReset().mockResolvedValue({});
  api.navigate.mockReset();
});
function panel() {
  render(
    <LocaleProvider>
      <div style={{ width: 900 }}>
        <AppsPanel onClose={() => {}} />
      </div>
    </LocaleProvider>,
  );
}

describe("Apps control panel", () => {
  it("searches and filters the app list and separates user actions from management", async () => {
    panel();
    await screen.findByRole("button", { name: /Alpha/ });
    await userEvent.type(screen.getByRole("textbox", { name: "Appを検索" }), "Draft");
    expect(screen.queryByRole("button", { name: /Alpha/ })).toBeNull();
    await userEvent.clear(screen.getByRole("textbox", { name: "Appを検索" }));
    await userEvent.selectOptions(
      screen.getByRole("combobox", { name: "Appの絞り込み" }),
      "installed",
    );
    expect(screen.queryByRole("button", { name: /Draft/ })).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: /Alpha/ }));
    expect(screen.queryByRole("button", { name: "App 管理画面を開く" })).toBeNull();
    await userEvent.click(screen.getByRole("tab", { name: "管理・開発" }));
    expect(screen.getByRole("button", { name: "App 管理画面を開く" })).toBeVisible();
    expect(screen.queryByRole("button", { name: "トークンを発行" })).toBeNull();
    // The exiting overview panel can coexist during the tab transition.
    const management = screen.getByRole("tabpanel", { name: "管理・開発" });
    await waitFor(() => expect(within(management).getByText("messages:write")).toBeVisible());
  });

  it("sends settings and creation to the app console", async () => {
    panel();
    await userEvent.click(await screen.findByRole("button", { name: "Appを作成" }));
    expect(api.navigate).toHaveBeenLastCalledWith({ to: "/apps" });
    await userEvent.click(screen.getByRole("button", { name: /Draft/ }));
    await userEvent.click(screen.getByRole("tab", { name: "管理・開発" }));
    await userEvent.click(screen.getByRole("button", { name: "App 管理画面を開く" }));
    expect(api.navigate).toHaveBeenLastCalledWith({
      to: "/apps/$appId",
      params: { appId: "draft" },
      search: { section: "basic" },
    });
    expect(api.request).not.toHaveBeenCalled();
  });

  it.each([
    {
      locale: "zh-Hant",
      search: "搜尋 App",
      manage: "管理與開發",
      console: "開啟 App 管理",
    },
    {
      locale: "zh-Hans",
      search: "搜索 App",
      manage: "管理与开发",
      console: "打开 App 管理",
    },
    {
      locale: "yue",
      search: "搜尋 App",
      manage: "管理同開發",
      console: "打開 App 管理",
    },
  ])("renders app management controls in $locale", async ({ locale, search, manage, console }) => {
    localStorage.setItem("zoo-locale", locale);
    panel();
    expect(screen.getByRole("textbox", { name: search })).toBeVisible();
    await userEvent.click(await screen.findByRole("button", { name: /Alpha/ }));
    await userEvent.click(screen.getByRole("tab", { name: manage }));
    expect(screen.getByRole("button", { name: console })).toBeVisible();
    expect(screen.queryByRole("button", { name: "Open app management" })).toBeNull();
  });

  it.each([null, "general"])("invokes global shortcuts with channel=%s", async (channel) => {
    api.selectedChannelId = channel;
    api.get.mockImplementation((path: string) =>
      Promise.resolve(
        path === "/api/apps"
          ? [{ ...apps[0], manifest: { ...apps[0]!.manifest, shortcuts: ["Global shortcut"] } }]
          : [],
      ),
    );
    panel();
    await userEvent.click(await screen.findByRole("button", { name: /Alpha/ }));
    await userEvent.click(screen.getByRole("button", { name: "Global shortcut" }));
    await waitFor(() =>
      expect(api.request).toHaveBeenCalledWith(
        "/api/apps/alpha/invoke",
        expect.objectContaining({
          body: expect.objectContaining({ kind: "shortcut", channel_id: channel }),
        }),
      ),
    );
  });

  it("offers retry after loading the catalog fails", async () => {
    api.get.mockRejectedValueOnce(new Error("offline"));
    panel();
    await screen.findByRole("alert");
    await userEvent.click(screen.getByRole("button", { name: "再試行" }));
    await waitFor(() => expect(screen.getByRole("button", { name: /Alpha/ })).toBeVisible());
    expect(screen.queryByRole("alert")).toBeNull();
  });
});
