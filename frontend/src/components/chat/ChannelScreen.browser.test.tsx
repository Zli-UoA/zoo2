import { render, screen } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vite-plus/test";

import { ChannelScreen } from "./ChannelScreen";

const state = vi.hoisted(() => ({
  query: {
    data: [{ id: "cached", channel_name: "Cached channel", is_dm: false }],
    isLoading: false,
    isInitialSyncLoading: true,
    isError: false,
  },
}));

vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));
vi.mock("../i18n/LocaleProvider", () => ({ useLocale: () => ({ locale: "ja" }) }));
vi.mock("../layout/shell-context", () => ({ useShell: () => ({ showSidebar: vi.fn() }) }));
vi.mock("../../hooks/useCurrentUser", () => ({ useCurrentUser: () => ({ currentUserId: "me" }) }));
vi.mock("../../hooks/useRealtime", () => ({
  useActiveChannels: () => state.query,
  useUsers: () => ({ data: [] }),
  useChannelAccess: () => ({ members: [], canRead: true, isMembershipPending: false }),
}));
vi.mock("./ChannelHeader", () => ({
  ChannelHeader: ({ channel }: { channel: { channel_name: string } }) => (
    <h1>{channel.channel_name}</h1>
  ),
}));
vi.mock("./messages/MessagePane", () => ({
  MessagePane: ({ channelId }: { channelId: string }) => <p>messages:{channelId}</p>,
}));

beforeEach(() => {
  state.query.data = [{ id: "cached", channel_name: "Cached channel", is_dm: false }];
  state.query.isInitialSyncLoading = true;
});

it("waits for initial sync when the requested channel is absent from the cache", () => {
  const { container, rerender } = render(<ChannelScreen channelId="new" />);
  expect(screen.queryByText("channels.notFound")).toBeNull();
  expect(container.querySelector('[data-slot="skeleton"]')).not.toBeNull();

  state.query.data = [
    ...state.query.data,
    { id: "new", channel_name: "New channel", is_dm: false },
  ];
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
  expect(screen.getByText("messages:cached")).toBeTruthy();
  expect(container.querySelector('[data-slot="skeleton"]')).toBeNull();
});
