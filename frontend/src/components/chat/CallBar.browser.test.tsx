import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vite-plus/test";

import type { CallParticipant } from "../../lib/call/types";
import type { ChannelRow } from "../../lib/electric/types";
import { LocaleProvider } from "../i18n/LocaleProvider";
import { CallBar } from "./CallBar";

// TanStack Router の Link をテスト用にモック
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children, className, ...props }: any) => (
    <a className={className} {...props}>
      {children}
    </a>
  ),
}));

const mockUsers = [
  { id: "user-alice", user_name: "alice", display_name: "Alice" },
  { id: "user-bob", user_name: "bob", display_name: "Bob" },
  { id: "user-charlie", user_name: "charlie", display_name: "Charlie" },
];

const mockMemberships = [
  { channel_id: "ch-dm", user_id: "user-alice" },
  { channel_id: "ch-dm", user_id: "user-bob" },
];

let mockCallState = {
  currentChannelId: "ch-1" as string | null,
  connectionState: "connected" as "idle" | "connecting" | "connected",
  isMuted: false,
  participants: [] as CallParticipant[],
  speakingUserIds: new Set<string>(),
  toggleMute: vi.fn(),
  leaveCall: vi.fn(),
};

vi.mock("../../lib/call/call-context", () => ({
  useCall: () => mockCallState,
}));

vi.mock("../../hooks/useCurrentUser", () => ({
  useCurrentUser: () => ({
    currentUserId: "user-alice",
    isLoading: false,
    isAdmin: false,
    error: null,
  }),
}));

vi.mock("../../lib/electric/collections-context", () => ({
  useOptionalCollections: () => ({
    users: {},
    channelUsers: {},
  }),
}));

vi.mock("@tanstack/react-db", () => ({
  useLiveQuery: (fn: any) => {
    const q = {
      from: (target: any) => {
        if (target.users) return { data: mockUsers };
        if (target.channelUsers) return { data: mockMemberships };
        return { data: [] };
      },
    };
    const res = fn(q);
    return res ?? { data: [] };
  },
}));

const channels: ChannelRow[] = [
  {
    id: "ch-1",
    channel_name: "general",
    description: null,
    created_user_id: "user-alice",
    is_private: false,
    is_dm: false,
    created_at: "2026-01-01 00:00:00.000",
    updated_at: null,
    archive_at: null,
    deleted_at: null,
  },
  {
    id: "ch-dm",
    channel_name: "dm:user-alice:user-bob",
    description: null,
    created_user_id: "user-alice",
    is_private: true,
    is_dm: true,
    created_at: "2026-01-01 00:00:00.000",
    updated_at: null,
    archive_at: null,
    deleted_at: null,
  },
];

function renderCallBar(customChannels: ChannelRow[] = channels) {
  return render(
    <LocaleProvider>
      <CallBar channels={customChannels} />
    </LocaleProvider>,
  );
}

describe("CallBar browser tests", () => {
  beforeEach(() => {
    localStorage.clear();
    localStorage.setItem("zoo-locale", "ja");
    mockCallState = {
      currentChannelId: "ch-1",
      connectionState: "connected",
      isMuted: false,
      participants: [
        {
          session_id: "00000000-0000-0000-0000-000000000001",
          user_id: "user-alice",
          user_name: "alice",
          display_name: "Alice",
          is_muted: false,
          joined_at: "2026-01-01T00:00:00Z",
        },
      ],
      speakingUserIds: new Set<string>(),
      toggleMute: vi.fn(),
      leaveCall: vi.fn(),
    };
  });

  it("idle 状態では何も描画されないこと", () => {
    mockCallState.connectionState = "idle";
    const { container } = renderCallBar();
    expect(container.querySelector("aside")).toBeNull();
  });

  it("connecting 状態で接続中インジケータと文言が表示されること", () => {
    mockCallState.connectionState = "connecting";
    renderCallBar();

    const aside = screen.getByRole("complementary", { name: "通話中" });
    expect(aside).toBeDefined();
    expect(screen.getByText("接続中…")).toBeDefined();
    expect(screen.getByText("general")).toBeDefined();
  });

  it("connected 状態でチャンネル名と参加者数が表示されること", () => {
    renderCallBar();

    expect(screen.getByText("general")).toBeDefined();
    expect(screen.getByText("1人が参加中")).toBeDefined();
    // アリスの頭文字 'A' のアバターが表示されていること
    expect(screen.getByText("A")).toBeDefined();
  });

  it("DM 通話時に内部チャンネル名ではなく相手の表示名が解決されて表示されること", () => {
    mockCallState.currentChannelId = "ch-dm";
    renderCallBar();

    // 内部名の "dm:user-alice:user-bob" ではなく、解決された相手の名前 "Bob" が表示されること
    expect(screen.queryByText("dm:user-alice:user-bob")).toBeNull();
    expect(screen.getByText("Bob")).toBeDefined();
  });

  it("ミュート状態の表示とトグル操作が動作すること", async () => {
    const user = userEvent.setup();
    mockCallState.isMuted = false;
    renderCallBar();

    const muteBtn = screen.getByRole("button", { name: "ミュート" });
    expect(muteBtn).toBeDefined();
    await user.click(muteBtn);
    expect(mockCallState.toggleMute).toHaveBeenCalledTimes(1);
  });

  it("ミュート中状態ではミュート解除ボタンが表示されること", async () => {
    const user = userEvent.setup();
    mockCallState.isMuted = true;
    renderCallBar();

    const unmuteBtn = screen.getByRole("button", { name: "ミュート解除" });
    expect(unmuteBtn).toBeDefined();
    await user.click(unmuteBtn);
    expect(mockCallState.toggleMute).toHaveBeenCalledTimes(1);
  });

  it("発話中ユーザーのアバターにリングスタイルが付与されること", () => {
    mockCallState.speakingUserIds = new Set(["user-alice"]);
    renderCallBar();

    const avatar = screen.getByTitle("Alice (発話中)");
    expect(avatar.className).toContain("ring-2 ring-emerald-500");
  });

  it("複数参加者のアバターが表示されること", () => {
    mockCallState.participants = [
      {
        session_id: "00000000-0000-0000-0000-000000000001",
        user_id: "user-alice",
        user_name: "alice",
        display_name: "Alice",
        is_muted: false,
        joined_at: "2026-01-01T00:00:00Z",
      },
      {
        session_id: "00000000-0000-0000-0000-000000000002",
        user_id: "user-bob",
        user_name: "bob",
        display_name: "Bob",
        is_muted: true,
        joined_at: "2026-01-01T00:00:01Z",
      },
      {
        session_id: "00000000-0000-0000-0000-000000000003",
        user_id: "user-charlie",
        user_name: "charlie",
        display_name: "Charlie",
        is_muted: false,
        joined_at: "2026-01-01T00:00:02Z",
      },
    ];

    renderCallBar();

    expect(screen.getByText("3人が参加中")).toBeDefined();
    expect(screen.getByText("A")).toBeDefined();
    expect(screen.getByText("B")).toBeDefined();
    expect(screen.getByText("C")).toBeDefined();

    // Bob はミュート中
    expect(screen.getByTitle("Bob (ミュート中)")).toBeDefined();
  });

  it("退出ボタンをクリックすると leaveCall が呼ばれること", async () => {
    const user = userEvent.setup();
    renderCallBar();

    const leaveBtn = screen.getByRole("button", { name: "通話から退出" });
    expect(leaveBtn).toBeDefined();
    await user.click(leaveBtn);
    expect(mockCallState.leaveCall).toHaveBeenCalledTimes(1);
  });
});
