import { act, render, screen } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vite-plus/test";

import type { AppConfig } from "../../lib/config";
import type { CurrentUserState } from "../../lib/currentUser";
import { OfflineProvider } from "./OfflineProvider";

const mocks = vi.hoisted(() => ({
  state: { currentUserId: null, isLoading: true, isAdmin: false, error: null } as CurrentUserState,
  listeners: new Set<() => void>(),
  online: true,
  open: vi.fn(async () => ({ persistence: {} })),
}));
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
  removeOfflineDatabase: async () => {},
  OfflinePersistenceUnavailableError: class extends Error {},
}));
vi.mock("../../lib/offline/identity", () => ({
  resolveOfflineNamespace: ({ currentUserId }: { currentUserId: string | null }) =>
    currentUserId ?? "anonymous",
  rememberOfflineUserId: vi.fn(),
  forgetOfflineUserId: vi.fn(),
  markOfflineDeletionPending: vi.fn(),
  clearPendingOfflineDeletion: vi.fn(),
}));
vi.mock("../layout/ShellSkeleton", () => ({ ShellSkeleton: () => <p>Loading</p> }));

beforeEach(() => {
  mocks.state = { currentUserId: null, isLoading: true, isAdmin: false, error: null };
  mocks.online = true;
  mocks.open.mockClear();
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
  expect(mocks.open).toHaveBeenCalledExactlyOnceWith("user-a");
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
