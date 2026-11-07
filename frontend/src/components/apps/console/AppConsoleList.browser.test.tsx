import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vite-plus/test";
import { LocaleProvider } from "../../i18n/LocaleProvider";
import { AppConsoleList, NEW_APP_MANIFEST } from "./AppConsoleList";
import "../../../styles.css";

const api = vi.hoisted(() => ({
  get: vi.fn(),
  request: vi.fn(),
  navigate: vi.fn(),
  admin: true,
}));
vi.mock("../../../lib/api/client", async (original) => ({
  ...(await original<typeof import("../../../lib/api/client")>()),
  createConfiguredApiClient: () => api,
}));
vi.mock("../../../lib/electric/collections-context", async (original) => ({
  ...(await original<typeof import("../../../lib/electric/collections-context")>()),
  useCollections: () => ({ config: {} }),
}));
vi.mock("../../../hooks/useCurrentUser", () => ({
  useCurrentUser: () => ({ currentUserId: "me", isAdmin: api.admin }),
}));
vi.mock("../../layout/shell-context", () => ({ useShell: () => ({ showSidebar: vi.fn() }) }));
vi.mock("@tanstack/react-router", async (original) => ({
  ...(await original<typeof import("@tanstack/react-router")>()),
  useNavigate: () => api.navigate,
  Link: ({ children }: { children: React.ReactNode }) => <a href="/">{children}</a>,
}));

const apps = [
  {
    id: "mine",
    name: "Mine",
    description: "",
    icon_url: null,
    owner_id: "me",
    status: "draft",
    can_manage: true,
    manifest: {},
  },
  {
    id: "review",
    name: "Review",
    description: "",
    icon_url: null,
    owner_id: "other",
    status: "pending",
    can_manage: true,
    manifest: {},
  },
  {
    id: "shared",
    name: "Shared",
    description: "",
    icon_url: null,
    owner_id: "other",
    status: "installed",
    can_manage: false,
    manifest: { notification_only: true },
  },
];
beforeEach(() => {
  api.admin = true;
  api.navigate.mockReset();
  api.get.mockReset().mockResolvedValue(apps);
  api.request.mockReset().mockResolvedValue({ id: "created" });
});
function list() {
  render(
    <LocaleProvider>
      <AppConsoleList />
    </LocaleProvider>,
  );
}

describe("App console list", () => {
  it("groups apps and only shows the approval queue to administrators", async () => {
    list();
    const pending = (await screen.findByRole("heading", { name: "承認待ち" })).closest("section")!;
    expect(within(pending).getByText("Review")).toBeVisible();
    const mine = screen.getByRole("heading", { name: "自分の App" }).closest("section")!;
    expect(within(mine).getByText("Mine")).toBeVisible();
    const installed = screen
      .getByRole("heading", { name: "インストール済みの App" })
      .closest("section")!;
    expect(installed.textContent).toContain("通知専用");
    expect(installed.textContent).toContain("閲覧のみ");
  });

  it("hides the approval queue from members", async () => {
    api.admin = false;
    list();
    await screen.findByRole("heading", { name: "自分の App" });
    expect(screen.queryByRole("heading", { name: "承認待ち" })).toBeNull();
  });

  it("creates an app and opens it in the console", async () => {
    list();
    await userEvent.click(await screen.findByRole("button", { name: "Appを作成" }));
    const dialog = await screen.findByRole("dialog");
    await userEvent.type(within(dialog).getByRole("textbox", { name: "App名" }), "Deploy bot");
    await userEvent.click(within(dialog).getByRole("button", { name: "Appを作成" }));
    await waitFor(() =>
      expect(api.request).toHaveBeenCalledWith("/api/apps", {
        method: "POST",
        body: { name: "Deploy bot", description: "", manifest: NEW_APP_MANIFEST },
      }),
    );
    expect(api.navigate).toHaveBeenCalledWith({
      to: "/apps/$appId",
      params: { appId: "created" },
      search: { section: "basic" },
    });
  });

  it("offers retry when the list cannot be loaded", async () => {
    api.get.mockRejectedValueOnce(new Error("offline"));
    list();
    await userEvent.click(await screen.findByRole("button", { name: "再試行" }));
    expect(await screen.findByText("Mine")).toBeVisible();
  });
});
