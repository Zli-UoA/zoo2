import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vite-plus/test";
import { LocaleProvider } from "../i18n/LocaleProvider";
import { WorkspaceMembers } from "./WorkspaceMembers";

const state = vi.hoisted(() => ({
  get: vi.fn(),
  request: vi.fn(),
  isAdmin: true,
  refresh: vi.fn(),
}));
vi.mock("../../lib/api/client", async (original) => ({
  ...(await original<typeof import("../../lib/api/client")>()),
  createConfiguredApiClient: () => state,
}));
vi.mock("../../lib/electric/collections-context", () => ({
  useCollections: () => ({ config: {} }),
}));
vi.mock("../../lib/currentUser", () => ({
  getCurrentUserStore: () => ({ refresh: state.refresh }),
}));
vi.mock("../../hooks/useCurrentUser", () => ({
  useCurrentUser: () => ({ currentUserId: "me", isAdmin: state.isAdmin }),
}));
vi.mock("../../hooks/useRealtime", () => ({
  useUsers: () => ({
    data: [
      { id: "me", user_name: "alice", display_name: "Alice" },
      { id: "bob", user_name: "bob", display_name: "Bob" },
    ],
    isLoading: false,
    isError: false,
  }),
}));
beforeEach(() => {
  state.isAdmin = true;
  state.get.mockReset().mockResolvedValue({ user_ids: ["me"], protected_user_ids: [] });
  state.request.mockReset().mockResolvedValue({});
});
function panel() {
  render(
    <LocaleProvider>
      <WorkspaceMembers />
    </LocaleProvider>,
  );
}

describe("workspace member management", () => {
  it("confirms administrator grants and prevents removing the last administrator", async () => {
    panel();
    await waitFor(() => expect(screen.getByRole("button", { name: "管理者にする" })).toBeEnabled());
    expect(screen.getByRole("button", { name: "管理者を外す" })).toBeDisabled();
    await userEvent.click(screen.getByRole("button", { name: "管理者にする" }));
    expect((await screen.findByRole("alertdialog")).textContent).toContain("Bob");
    await userEvent.click(screen.getAllByRole("button", { name: "管理者にする" }).at(-1)!);
    await waitFor(() =>
      expect(state.request).toHaveBeenCalledWith("/api/users/bob/admin", {
        method: "PATCH",
        body: { is_admin: true },
      }),
    );
    await waitFor(() =>
      expect(screen.getAllByRole("button", { name: "管理者を外す" })).toHaveLength(2),
    );
  });
  it("shows members without role actions to non-administrators", async () => {
    state.isAdmin = false;
    panel();
    await screen.findByText("Bob");
    expect(screen.queryByRole("button", { name: "管理者にする" })).toBeNull();
    expect(screen.queryByRole("button", { name: "管理者を外す" })).toBeNull();
  });
  it("keeps failed role changes open for retry", async () => {
    state.request.mockRejectedValue(new Error("offline"));
    panel();
    await waitFor(() => expect(screen.getByRole("button", { name: "管理者にする" })).toBeEnabled());
    await userEvent.click(screen.getByRole("button", { name: "管理者にする" }));
    await userEvent.click(screen.getAllByRole("button", { name: "管理者にする" }).at(-1)!);
    await screen.findByRole("alert");
    expect(screen.getByRole("alertdialog")).toBeVisible();
  });
});
