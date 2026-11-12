import { render, screen } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vite-plus/test";
import { LocaleProvider } from "../i18n/LocaleProvider";
import { ChannelSidebar } from "./ChannelSidebar";
import type { ChannelRow } from "../../lib/electric/types";

const fixture = vi.hoisted(() => ({
  memberships: [{ user_id: "other", channel_id: "general" }],
}));
vi.mock("@tanstack/react-db", () => ({
  useLiveQuery: () => ({
    data: fixture.memberships,
    isLoading: true,
    isPersistedReady: true,
    isError: false,
  }),
}));
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children }: { children: import("react").ReactNode }) => (
    <a href="/channels/general">{children}</a>
  ),
}));
vi.mock("../../hooks/useCurrentUser", () => ({
  useCurrentUser: () => ({ currentUserId: "me", isLoading: false }),
}));
vi.mock("../../hooks/useRealtime", () => ({ useUsers: () => ({ data: [], isLoading: false }) }));
vi.mock("../../hooks/useUnreadChannels", () => ({ useUnreadChannels: () => new Set() }));
vi.mock("../../lib/electric/collections-context", () => ({
  useCollections: () => ({ channelUsers: {} }),
}));
vi.mock("../../lib/call/call-context", () => ({ useCall: () => ({ activeCalls: new Map() }) }));
vi.mock("../layout/shell-context", () => ({
  useShell: () => ({ selectedChannelId: null, showChat: vi.fn() }),
}));
vi.mock("../SyncStatus", () => ({ SyncStatus: () => null }));
vi.mock("./ChannelBrowserDialog", () => ({ ChannelBrowserDialog: () => null }));
vi.mock("./CreateChannelDialog", () => ({ CreateChannelDialog: () => null }));
vi.mock("./CreateDmDialog", () => ({ CreateDmDialog: () => null }));
vi.mock("./DmSection", () => ({ DmSection: () => null }));

beforeEach(() => {
  fixture.memberships = [{ user_id: "other", channel_id: "general" }];
});

it("waits for the current user's membership even when other users are cached", () => {
  const channel = { id: "general", channel_name: "general", is_dm: false } as ChannelRow;
  const view = () => (
    <LocaleProvider>
      <ChannelSidebar channels={[channel]} loading={false} error={false} />
    </LocaleProvider>
  );
  const { container, rerender } = render(view());
  expect(container.querySelector('[data-slot="skeleton"]')).not.toBeNull();
  expect(screen.queryByText("参加中のチャンネルはありません")).toBeNull();
  fixture.memberships = [...fixture.memberships, { user_id: "me", channel_id: "general" }];
  rerender(view());
  expect(screen.getByRole("link", { name: "general" })).toBeTruthy();
  expect(container.querySelector('[data-slot="skeleton"]')).toBeNull();
});
