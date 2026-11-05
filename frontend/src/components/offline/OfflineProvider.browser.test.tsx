import { act, render, screen } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vite-plus/test";

import type { AppConfig } from "../../lib/config";
import type { CurrentUserState } from "../../lib/currentUser";
import { useOfflineStatus } from "../../lib/offline/offline-context";
import { OfflineProvider } from "./OfflineProvider";

const mocks = vi.hoisted(() => ({
  state: { currentUserId: null, isLoading: true, isAdmin: false, error: null } as CurrentUserState,
  listeners: new Set<() => void>(),
  online: true,
  prepare: vi.fn(async () => {}),
  open: vi.fn(async () => ({ persistence: {} })),
  remove: vi.fn(async () => {}),
}));
vi.mock("../../lib/pwa/worker-registration", () => ({ prepareOfflineWorker: mocks.prepare }));
vi.mock("../../lib/offline/online", () => ({ useOnlineStatus: () => mocks.online }));
vi.mock("../../lib/currentUser", () => ({
  getCurrentUserStore: () => ({
    getState: () => mocks.state,
    subscribe: (listener: () => void) => {
      mocks.listeners.add(listener);
      return () => mocks.listeners.delete(listener);
    },
    ensureLoaded: async () => {},
  }),
}));
vi.mock("../../lib/electric/collections", () => ({ getCollections: () => ({}) }));
vi.mock("../../lib/offline/persistence", () => ({
  openOfflinePersistence: mocks.open,
  retryPendingOfflineDeletion: async () => {},
  removeOfflineDatabase: mocks.remove,
  OfflinePersistenceUnavailableError: class extends Error {},
}));
vi.mock("../layout/ShellSkeleton", () => ({ ShellSkeleton: () => <p>Loading</p> }));

beforeEach(() => {
  mocks.state = { currentUserId: null, isLoading: true, isAdmin: false, error: null };
  mocks.online = true;
  mocks.open.mockClear();
  mocks.remove.mockReset();
  mocks.remove.mockResolvedValue(undefined);
  mocks.prepare.mockReset();
  mocks.prepare.mockResolvedValue(undefined);
});

it("waits for user resolution before opening storage and mounting interactive children", async () => {
  render(
    <OfflineProvider config={{ devUserId: null, oidc: {} } as AppConfig}>
      <button>Create channel</button>
    </OfflineProvider>,
  );
  await act(async () => {});
  expect(screen.queryByRole("button")).toBeNull();
  expect(mocks.open).not.toHaveBeenCalled();

  await act(async () => {
    mocks.state = { currentUserId: "user-a", isLoading: false, isAdmin: false, error: null };
    for (const listener of mocks.listeners) listener();
  });
  expect(await screen.findByRole("button", { name: "Create channel" })).toBeTruthy();
  expect(mocks.open).toHaveBeenCalledExactlyOnceWith("user-user-a");
});

it.each([
  { online: false, oidc: {} },
  { online: true, oidc: null },
])("does not block cached or dev startup: %j", async ({ online, oidc }) => {
  mocks.online = online;
  render(
    <OfflineProvider config={{ devUserId: null, oidc } as AppConfig}>
      <button>Cached channel</button>
    </OfflineProvider>,
  );
  expect(await screen.findByRole("button", { name: "Cached channel" })).toBeTruthy();
  expect(mocks.open).toHaveBeenCalledExactlyOnceWith("anonymous");
});

it("does not open the previous user's cache after online user resolution fails", async () => {
  localStorage.setItem("zoo.offline.lastUser", "previous-user");
  mocks.state = { ...mocks.state, isLoading: false, error: "Request timed out" };
  render(
    <OfflineProvider config={{ devUserId: null, oidc: {} } as AppConfig}>
      <button>App</button>
    </OfflineProvider>,
  );
  await screen.findByRole("alert");
  expect(screen.queryByRole("button", { name: "App" })).toBeNull();
  expect(mocks.open).not.toHaveBeenCalled();
});
it("keeps the last-user cache available when offline", async () => {
  localStorage.setItem("zoo.offline.lastUser", "previous-user");
  mocks.online = false;
  render(
    <OfflineProvider config={{ devUserId: null, oidc: {} } as AppConfig}>
      <button>Cache</button>
    </OfflineProvider>,
  );
  await screen.findByRole("button", { name: "Cache" });
  expect(mocks.open).toHaveBeenCalledExactlyOnceWith("user-previous-user");
});

it("waits for the controlling Service Worker update before opening SQLite", async () => {
  let complete!: () => void;
  mocks.prepare.mockReturnValueOnce(
    new Promise<void>((resolve) => {
      complete = resolve;
    }),
  );
  render(
    <OfflineProvider config={{ devUserId: "dev", oidc: null } as AppConfig}>
      <button>Ready</button>
    </OfflineProvider>,
  );
  await act(async () => {});
  expect(mocks.prepare).toHaveBeenCalledTimes(1);
  expect(mocks.open).not.toHaveBeenCalled();
  await act(async () => complete());
  await screen.findByRole("button", { name: "Ready" });
  expect(mocks.open).toHaveBeenCalledTimes(1);
});

function ClearUserData() {
  const { clearUserData } = useOfflineStatus();
  return <button onClick={() => void clearUserData().catch(() => {})}>Log out</button>;
}
it.each([false, true])(
  "React logout clears only this user's Compose drafts even if OPFS deletion fails: %s",
  async (fails) => {
    mocks.state = { currentUserId: "user-a", isLoading: false, isAdmin: false, error: null };
    localStorage.setItem("zoo-compose:user-a:draft:private:", "private draft");
    localStorage.setItem("zoo-compose:user-ab:draft:private:", "other user's draft");
    if (fails) mocks.remove.mockRejectedValueOnce(new Error("OPFS unavailable"));
    render(
      <OfflineProvider config={{ devUserId: null, oidc: {} } as AppConfig}>
        <ClearUserData />
      </OfflineProvider>,
    );
    const button = await screen.findByRole("button", { name: "Log out" });
    await act(async () => button.click());
    expect(mocks.remove).toHaveBeenCalledExactlyOnceWith("user-user-a");
    expect(localStorage.getItem("zoo-compose:user-a:draft:private:")).toBeNull();
    expect(localStorage.getItem("zoo-compose:user-ab:draft:private:")).toBe("other user's draft");
  },
);
