import { act, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vite-plus/test";

import { DEFAULT_CONFIG } from "../../lib/config";
import type { CurrentUserState } from "../../lib/currentUser";
import { OfflineProvider } from "./OfflineProvider";

const mocks = vi.hoisted(() => ({
  getCurrentUserStore: vi.fn(),
  getCollections: vi.fn(),
  openOfflinePersistence: vi.fn(),
  useOnlineStatus: vi.fn(),
}));
vi.mock("../../lib/currentUser", () => ({ getCurrentUserStore: mocks.getCurrentUserStore }));
vi.mock("../../lib/electric/collections", () => ({ getCollections: mocks.getCollections }));
vi.mock("../../lib/offline/online", () => ({ useOnlineStatus: mocks.useOnlineStatus }));
vi.mock("../../lib/offline/persistence", () => ({
  openOfflinePersistence: mocks.openOfflinePersistence,
  retryPendingOfflineDeletion: async () => {},
  removeOfflineDatabase: async () => {},
  OfflinePersistenceUnavailableError: class extends Error {},
}));
vi.mock("../layout/ShellSkeleton", () => ({ ShellSkeleton: () => <span>Loading shell</span> }));
const config = {
  ...DEFAULT_CONFIG,
  oidc: {
    issuer: "https://id.example",
    clientId: "zoo",
    redirectUri: null,
    scope: "openid profile",
    resource: null,
    postLogoutRedirectUri: null,
  },
};
let state: CurrentUserState;
let listeners: Set<() => void>;
beforeEach(() => {
  state = { currentUserId: null, isLoading: true, isAdmin: false, error: null };
  listeners = new Set();
  mocks.getCurrentUserStore.mockReturnValue({
    getState: () => state,
    subscribe: (listener: () => void) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    ensureLoaded: async () => {},
  });
  mocks.getCollections.mockReturnValue({ config });
  mocks.openOfflinePersistence.mockResolvedValue({ persistence: {} });
  mocks.useOnlineStatus.mockReturnValue(true);
});
afterEach(() => vi.clearAllMocks());
it("does not mount an anonymous shell before the online OIDC user resolves", async () => {
  render(
    <OfflineProvider config={config}>
      <input aria-label="channel name" />
    </OfflineProvider>,
  );
  await act(async () => {});
  expect(screen.queryByRole("textbox")).toBeNull();
  expect(mocks.openOfflinePersistence).not.toHaveBeenCalled();
  act(() => {
    state = { ...state, currentUserId: "alice", isLoading: false };
    for (const listener of listeners) listener();
  });
  await screen.findByRole("textbox");
  expect(mocks.openOfflinePersistence).toHaveBeenCalledWith("user-alice");
  expect(mocks.getCollections).toHaveBeenCalledTimes(1);
});
it("opens the last user's cache offline while user resolution is pending", async () => {
  localStorage.setItem("zoo.offline.lastUser", "alice");
  mocks.useOnlineStatus.mockReturnValue(false);
  render(
    <OfflineProvider config={config}>
      <input aria-label="channel name" />
    </OfflineProvider>,
  );
  await screen.findByRole("textbox");
  expect(mocks.openOfflinePersistence).toHaveBeenCalledWith("user-alice");
});
