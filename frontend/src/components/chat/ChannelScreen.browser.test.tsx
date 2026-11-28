import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vite-plus/test";

import { ChannelScreen } from "./ChannelScreen";

function channel(overrides: Record<string, unknown> = {}) {
  return {
    id: "cached",
    channel_name: "Cached channel",
    is_dm: false,
    is_private: false,
    encrypted: false,
    posting_policy: "members",
    archive_at: null,
    ...overrides,
  };
}

function access(overrides: Record<string, unknown> = {}) {
  return {
    members: [],
    canRead: true,
    isMembershipPending: false,
    isMember: true,
    isArchived: false,
    canManageChannel: false,
    isSyncing: false,
    ...overrides,
  };
}

const state = vi.hoisted(() => ({
  query: {
    data: [] as Record<string, unknown>[],
    isLoading: false,
    isInitialSyncLoading: true,
    isError: false,
  },
  access: {} as Record<string, unknown>,
}));

vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));
vi.mock("../i18n/LocaleProvider", () => ({ useLocale: () => ({ locale: "ja" }) }));
vi.mock("../layout/shell-context", () => ({ useShell: () => ({ showSidebar: vi.fn() }) }));
vi.mock("../../hooks/useCurrentUser", () => ({ useCurrentUser: () => ({ currentUserId: "me" }) }));
vi.mock("../../hooks/useRealtime", () => ({
  useActiveChannels: () => state.query,
  useUsers: () => ({ data: [] }),
  useAllUsers: () => ({ data: [] }),
  useChannelAccess: () => state.access,
}));
vi.mock("./ChannelHeader", () => ({
  ChannelHeader: ({ channel }: { channel: { channel_name: string } }) => (
    <h1>{channel.channel_name}</h1>
  ),
}));
vi.mock("./messages/MessagePane", () => ({
  MessagePane: ({ channelId, isActive }: { channelId: string; isActive?: boolean }) => (
    <p>
      messages:{channelId}:{String(isActive)}
    </p>
  ),
}));
vi.mock("./notes/ChannelNotesScreen", () => ({
  ChannelNotesScreen: (props: {
    channelId: string;
    canRead: boolean;
    canWrite: boolean;
    isMember: boolean;
    isArchived: boolean;
  }) => (
    <p>
      notes:{props.channelId}:{String(props.canRead)}:{String(props.canWrite)}:
      {String(props.isMember)}:{String(props.isArchived)}
    </p>
  ),
}));

beforeEach(() => {
  state.query.data = [channel()];
  state.query.isInitialSyncLoading = true;
  state.access = access();
});

it("waits for initial sync when the requested channel is absent from the cache", () => {
  const { container, rerender } = render(<ChannelScreen channelId="new" />);
  expect(screen.queryByText("channels.notFound")).toBeNull();
  expect(container.querySelector('[data-slot="skeleton"]')).not.toBeNull();

  state.query.data = [...state.query.data, channel({ id: "new", channel_name: "New channel" })];
  state.query.isInitialSyncLoading = false;
  rerender(<ChannelScreen channelId="new" />);
  expect(screen.getByRole("heading", { name: "New channel" })).toBeTruthy();
  expect(container.querySelector('[data-slot="skeleton"]')).toBeNull();
});

it("confirms a missing channel only after initial sync finishes", () => {
  const { rerender } = render(<ChannelScreen channelId="missing" />);
  expect(screen.queryByText("channels.notFound")).toBeNull();
  state.query.isInitialSyncLoading = false;
  rerender(<ChannelScreen channelId="missing" />);
  expect(screen.getByText("channels.notFound")).toBeTruthy();
});

it("renders a cached channel while initial sync is still pending", () => {
  const { container } = render(<ChannelScreen channelId="cached" />);
  expect(screen.getByRole("heading", { name: "Cached channel" })).toBeTruthy();
  expect(screen.getByText("messages:cached:true")).toBeTruthy();
  expect(container.querySelector('[data-slot="skeleton"]')).toBeNull();
});

it("shows notes for a public member and does not apply the message posting policy", async () => {
  state.query.data = [channel({ posting_policy: "managers" })];
  state.query.isInitialSyncLoading = false;
  render(<ChannelScreen channelId="cached" />);

  await userEvent.click(screen.getByRole("tab", { name: "notes.title" }));
  expect(screen.getByText("notes:cached:true:true:true:false")).toBeTruthy();
  expect(screen.getByText("messages:cached:false")).toBeTruthy();
});

it("keeps notes read-only for an archived member", async () => {
  state.query.data = [channel({ archive_at: "2026-11-28T00:00:00.000" })];
  state.query.isInitialSyncLoading = false;
  state.access = access({ isArchived: true });
  render(<ChannelScreen channelId="cached" />);

  await userEvent.click(screen.getByRole("tab", { name: "notes.title" }));
  expect(screen.getByText("notes:cached:true:false:true:true")).toBeTruthy();
});

it("shows read-only notes to a non-member of a public channel", async () => {
  state.query.isInitialSyncLoading = false;
  state.access = access({ isMember: false });
  render(<ChannelScreen channelId="cached" />);

  await userEvent.click(screen.getByRole("tab", { name: "notes.title" }));
  expect(screen.getByText("notes:cached:true:false:false:false")).toBeTruthy();
});

it("does not show notes for a DM", () => {
  state.query.data = [channel({ is_dm: true })];
  state.query.isInitialSyncLoading = false;
  render(<ChannelScreen channelId="cached" />);
  expect(screen.queryByRole("tab", { name: "notes.title" })).toBeNull();
});

it("does not show notes for an encrypted channel", () => {
  state.query.data = [channel({ encrypted: true, is_private: true })];
  state.query.isInitialSyncLoading = false;
  render(<ChannelScreen channelId="cached" />);
  expect(screen.queryByRole("tab", { name: "notes.title" })).toBeNull();
});

it("does not show notes when the channel is not readable", () => {
  state.query.data = [channel({ is_private: true })];
  state.query.isInitialSyncLoading = false;
  state.access = access({ canRead: false, isMember: false });
  render(<ChannelScreen channelId="cached" />);
  expect(screen.queryByRole("tab", { name: "notes.title" })).toBeNull();
  expect(screen.getByText("channels.privateMessages")).toBeTruthy();
});
