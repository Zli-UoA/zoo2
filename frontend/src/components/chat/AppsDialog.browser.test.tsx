import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vite-plus/test";
import { LocaleProvider } from "../i18n/LocaleProvider";
import { AppsPanel } from "./AppsDialog";
import "../../styles.css";

const api = vi.hoisted(() => ({ get: vi.fn(), request: vi.fn() }));
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
vi.mock("../layout/shell-context", () => ({ useShell: () => ({ selectedChannelId: "general" }) }));
vi.mock("@tanstack/react-router", () => ({ useNavigate: () => vi.fn() }));
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
  api.get
    .mockReset()
    .mockImplementation((path: string) => Promise.resolve(path === "/api/apps" ? apps : []));
  api.request.mockReset().mockResolvedValue({});
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
    expect(screen.queryByRole("button", { name: "トークンを発行" })).toBeNull();
    await userEvent.click(screen.getByRole("tab", { name: "管理・開発" }));
    expect(screen.getByRole("button", { name: "トークンを発行" })).toBeVisible();
    expect(screen.queryByRole("button", { name: "承認を申請" })).toBeNull();
    expect(screen.getByText("messages:write")).toBeVisible();
  });

  it("lets users cancel editing and reports malformed manifests before sending", async () => {
    panel();
    await userEvent.click(await screen.findByRole("button", { name: /Draft/ }));
    await userEvent.click(screen.getByRole("tab", { name: "管理・開発" }));
    await userEvent.click(screen.getByRole("button", { name: "編集" }));
    const manifest = screen.getByRole("textbox", { name: "App設定（JSON）" });
    await userEvent.clear(manifest);
    await userEvent.type(manifest, "invalid JSON");
    await userEvent.click(screen.getByRole("button", { name: "保存" }));
    expect(screen.getByRole("alert")).toHaveTextContent("Manifestに有効なJSONを入力してください。");
    expect(api.request).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: "キャンセル" }));
    expect(screen.queryByRole("textbox", { name: "App設定（JSON）" })).toBeNull();
  });

  it("confirms deletion and removes the deleted app from the catalog", async () => {
    let current = [...apps];
    api.get.mockImplementation((path: string) =>
      Promise.resolve(path === "/api/apps" ? current : []),
    );
    api.request.mockImplementation(async (path: string) => {
      if (path === "/api/apps/draft/actions/delete")
        current = current.filter((entry) => entry.id !== "draft");
      return {};
    });
    panel();
    await userEvent.click(await screen.findByRole("button", { name: /Draft/ }));
    await userEvent.click(screen.getByRole("tab", { name: "管理・開発" }));
    await userEvent.click(screen.getByRole("button", { name: "Appを削除" }));
    expect((await screen.findByRole("alertdialog")).textContent).toContain(
      "「Draft」を削除しますか？",
    );
    await userEvent.click(screen.getByRole("button", { name: "キャンセル" }));
    expect(api.request).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: "Appを削除" }));
    await userEvent.click(screen.getAllByRole("button", { name: "Appを削除" }).at(-1)!);
    await waitFor(() =>
      expect(api.request).toHaveBeenCalledWith("/api/apps/draft/actions/delete", {
        method: "POST",
      }),
    );
    await waitFor(() => expect(screen.queryByRole("button", { name: /Draft/ })).toBeNull());
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
