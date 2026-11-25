import "../../../styles.css";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  Outlet,
  RouterProvider,
  useRouterState,
} from "@tanstack/react-router";
import { useState, type ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vite-plus/test";

import { createStoryCollections } from "../../../../.storybook/fixtures";
import {
  ENCRYPTED_CHANNEL_STORAGE_PREFIX,
  noteEncryptedChannel,
  resetSeenEncryptedChannels,
} from "../../../lib/e2ee/encryptionGuard";
import { E2eeContext, type E2eeContextValue } from "../../../lib/e2ee/e2ee-context";
import { LOCALE_STORAGE_KEY } from "../../../lib/i18n/locale";
import { CollectionsProvider } from "../../../lib/electric/collections-context";
import type { ChannelRow, MessageRow, UserRow } from "../../../lib/electric/types";
import { LocaleProvider } from "../../i18n/LocaleProvider";
import type { ForwardEntryState } from "../../../hooks/useForwardedMessages";
import { ForwardedMessageCard } from "./ForwardedMessageCard";
import { MessageBody } from "./MessageRow";
import { MfmChannelsProvider } from "./MfmChannels";
import { MfmContent } from "./MfmContent";
import { LinkPreviews } from "./LinkPreviews";

const summaly = vi.hoisted(() => ({ get: vi.fn() }));
vi.mock("../../../lib/summaly", () => ({ createSummalyClient: () => summaly }));
vi.mock("../../../lib/preview-endpoints", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../lib/preview-endpoints")>()),
  subscribePreviewEndpoints: (
    _config: unknown,
    listener: (endpoints: { summaly: string; media: string }) => void,
  ) => {
    listener({ summaly: "https://summaly.example/url", media: "https://media.example/" });
    return () => {};
  },
}));

const origin = window.location.origin;
const ts = (minute: number) => `2026-11-23 07:${String(minute).padStart(2, "0")}:00.000`;

function channel(overrides: Partial<ChannelRow> & Pick<ChannelRow, "id">): ChannelRow {
  return {
    channel_name: overrides.id,
    description: null,
    created_user_id: "u-alice",
    is_private: false,
    is_dm: false,
    created_at: ts(0),
    updated_at: null,
    archive_at: null,
    deleted_at: null,
    imported_from_slack: false,
    slack_import_kind: null,
    slack_imported_at: null,
    ...overrides,
  };
}

function message(
  overrides: Partial<MessageRow> & Pick<MessageRow, "id" | "channel_id">,
): MessageRow {
  return {
    user_id: "u-bob",
    content: "テスト",
    created_at: ts(30),
    updated_at: null,
    deleted_at: null,
    parent_id: null,
    ...overrides,
  };
}

const users: UserRow[] = [
  { id: "u-alice", user_name: "alice", display_name: "Alice", created_at: ts(0), updated_at: null },
  { id: "u-bob", user_name: "bob", display_name: "アメシュ", created_at: ts(0), updated_at: null },
];

const general = channel({ id: "c-general", channel_name: "general", description: "全体の連絡" });
const secretMember = channel({ id: "c-secret", channel_name: "secret", is_private: true });
const sealed = channel({
  id: "c-sealed",
  channel_name: "sealed",
  is_private: true,
  encrypted: true,
});
const dm = channel({
  id: "c-dm",
  channel_name: "dm:u-alice:u-bob",
  is_private: true,
  is_dm: true,
});

/** サーバーが `encrypted = false` を配る、暗号化チャンネルだったチャンネル（格下げ）。 */
const edm = channel({
  id: "c-edm",
  channel_name: "edm:u-alice:u-bob",
  is_private: true,
  is_dm: true,
});
const recorded = channel({ id: "c-recorded", channel_name: "recorded", is_private: true });
/** `encrypted = false` が配られているが、暗号文のある行を含むチャンネル（格下げ）。 */
const downgraded = channel({ id: "c-downgraded", channel_name: "downgraded" });

const ciphertext = {
  algorithm: "m.megolm.v1.aes-sha2" as const,
  sender_key: "key",
  device_id: "device",
  session_id: "session",
  ciphertext: "AwgA",
};
const cardJson = JSON.stringify({
  title: "デプロイ完了",
  description: "",
  url: "https://example.com",
  source: "CI",
  context: "",
  author: "",
  badge: "",
});

const messages: MessageRow[] = [
  message({ id: "m-cipher", channel_id: "c-downgraded", content: "", ciphertext }),
  message({
    id: "m-plain-sibling",
    channel_id: "c-downgraded",
    content: "暗号文の隣の平文",
    created_at: ts(31),
  }),
  message({ id: "m-edm", channel_id: "c-edm", content: "edm の平文" }),
  message({ id: "m-recorded", channel_id: "c-recorded", content: "記録済みの平文" }),
  message({
    id: "m-card",
    channel_id: "c-general",
    content: "カードの代わりの本文",
    card: cardJson,
  }),
  message({
    id: "m-parts",
    channel_id: "c-general",
    content: "部品の代わりの本文",
    components: JSON.stringify({ flags: 1 << 15, components: [{ type: 10, content: "部品" }] }),
  }),
  message({ id: "m-general", channel_id: "c-general", content: "テスト" }),
  message({
    id: "m-reply",
    channel_id: "c-general",
    content: "返信です",
    parent_id: "m-general",
    created_at: ts(31),
  }),
  message({ id: "m-secret", channel_id: "c-secret", content: "参加者だけの話" }),
  message({ id: "m-sealed", channel_id: "c-sealed", content: "暗号化された本文は出さない" }),
  message({ id: "m-dm", channel_id: "c-dm", content: "DM の内容" }),
  message({
    id: "m-deleted",
    channel_id: "c-general",
    content: "消えた本文",
    deleted_at: ts(40),
  }),
  message({
    id: "m-files",
    channel_id: "c-general",
    content: "",
    display_data: {
      emojis: {},
      attachments: [{ id: "a1" }, { id: "a2" }] as never,
      reactions: [],
      reply_count: 0,
    },
  }),
  message({
    id: "m-forward-only",
    channel_id: "c-general",
    content: "",
    forwarded_message_ids: ["m-general"],
  }),
  message({
    id: "m-forward-comment",
    channel_id: "c-general",
    content: "これを見てください",
    forwarded_message_ids: ["m-general"],
  }),
  message({
    id: "m-long",
    channel_id: "c-general",
    content: "とても長い本文です。".repeat(80),
  }),
  message({
    id: "m-md",
    channel_id: "c-general",
    content: "**太字**と[リンク](https://example.com/a)と`code`",
  }),
];

type WorkerRequest = (request: { op: string; channelId: string }) => Promise<unknown>;

/** 暗号ストアを開いたタブ（`worker` がある）を再現する。値は 1 度だけ作り、描き直しで変えない。 */
function cryptoStore(request: WorkerRequest): E2eeContextValue {
  return { worker: { request } } as unknown as E2eeContextValue;
}

/** `known` の答えを返す暗号ストア。記録の依頼（`megolmRememberEncrypted`）は受けるだけ。 */
function workerKnowing(known: (channelId: string) => Promise<boolean>) {
  return vi.fn<WorkerRequest>(({ op, channelId }) =>
    op === "megolmIsKnownEncrypted" ? known(channelId) : Promise.resolve(undefined),
  );
}

interface SetupOptions {
  content: string;
  /** メッセージ取得の対象にできるチャンネル（ユーザーが見られるもの）。 */
  visible?: ChannelRow[];
  ready?: boolean;
  /** 一覧には出ないが、コレクションには行がある（サーバーの権限で守られる）チャンネル。 */
  hiddenRows?: MessageRow[];
  previews?: "auto" | "manual" | "none";
  /** 指定すると、本文の代わりに、これを描く（`content` は使わない）。 */
  node?: ReactNode;
  /** `false` で、チャンネルの一覧（Provider）を置かない（公開プロフィールなどを想定）。 */
  directory?: boolean;
  highlight?: string[];
  body?: "message" | "content";
  /** 暗号ストアを開いたタブにする。省くと、`E2eeProvider` のない状態（`worker` がない）。 */
  worker?: WorkerRequest;
  /** `E2eeProvider` が配る値を直接指定する。 */
  e2eeValue?: E2eeContextValue;
}

function setup({
  content,
  visible = [general, secretMember, sealed, dm, downgraded],
  ready = true,
  hiddenRows = [],
  previews = "auto",
  directory = true,
  highlight,
  body = "message",
  worker,
  e2eeValue,
  node,
}: SetupOptions) {
  const initialE2ee = e2eeValue ?? (worker ? cryptoStore(worker) : null);
  const e2eeControl: { set: ((next: E2eeContextValue) => void) | null } = { set: null };
  const bundle = createStoryCollections({
    currentUserId: "u-alice",
    users,
    // コレクションには、一覧に出るかどうかによらず、すべてのチャンネルを置く（あとから一覧に現れるものも含む）。
    // 見られるかどうかを決めるのは、一覧（`MfmChannelsProvider`）で、コレクションではない。
    channels: [
      general,
      secretMember,
      sealed,
      dm,
      edm,
      recorded,
      downgraded,
      channel({ id: "c-hidden", channel_name: "秘密の計画", is_private: true }),
    ],
    messages: [...messages, ...hiddenRows],
  });
  const original = bundle.messages;
  const messagesFor = vi.fn((channelId: string) => original(channelId));
  bundle.messages = messagesFor as typeof bundle.messages;

  const directoryControl: {
    set: ((next: { visible: ChannelRow[]; ready: boolean }) => void) | null;
  } = { set: null };
  const root = createRootRoute({ component: Outlet });
  function Body() {
    const [e2ee, setE2ee] = useState(initialE2ee);
    e2eeControl.set = setE2ee;
    const [state, setState] = useState({ visible, ready });
    directoryControl.set = setState;
    const rendered =
      node ??
      (body === "message" ? (
        <MessageBody content={content} previews={previews} />
      ) : (
        <MfmContent content={content} highlight={highlight} />
      ));
    const tree = (
      <LocaleProvider>
        <CollectionsProvider value={bundle}>
          {directory ? (
            <MfmChannelsProvider channels={state.visible} ready={state.ready}>
              {rendered}
            </MfmChannelsProvider>
          ) : (
            rendered
          )}
        </CollectionsProvider>
      </LocaleProvider>
    );
    return e2ee ? <E2eeContext.Provider value={e2ee}>{tree}</E2eeContext.Provider> : tree;
  }
  const home = createRoute({ getParentRoute: () => root, path: "/", component: Body });
  const channelRoute = createRoute({
    getParentRoute: () => root,
    path: "/channels/$channelId",
    validateSearch: (search: Record<string, unknown>): { message?: string; thread?: string } => ({
      message: typeof search.message === "string" ? search.message : undefined,
      thread: typeof search.thread === "string" ? search.thread : undefined,
    }),
    component: function Opened() {
      const location = useRouterState({ select: (state) => state.location });
      const { message: m, thread } = location.search as { message?: string; thread?: string };
      return (
        <output aria-label="Opened">
          {[location.pathname.split("/").at(-1), m ?? "", thread ?? ""].join("|")}
        </output>
      );
    },
  });
  const router = createRouter({
    routeTree: root.addChildren([home, channelRoute]),
    history: createMemoryHistory({ initialEntries: ["/"] }),
  });
  render(<RouterProvider router={router} />);
  return {
    router,
    messagesFor,
    /** チャンネルの一覧（見られるチャンネルと、同期が終わったか）を、あとから差し替える。 */
    /** `E2eeProvider` が配る値を、あとから差し替える（暗号ストアを開き終えたなど）。 */
    updateCryptoStore: async (next: E2eeContextValue) => {
      await act(async () => {
        e2eeControl.set?.(next);
      });
    },
    updateDirectory: async (next: { visible: ChannelRow[]; ready: boolean }) => {
      await act(async () => {
        directoryControl.set?.(next);
      });
    },
  };
}

const link = (channelId: string, search = "") => `${origin}/channels/${channelId}${search}`;

beforeEach(() => {
  localStorage.setItem(LOCALE_STORAGE_KEY, "ja");
  summaly.get.mockReset();
  summaly.get.mockResolvedValue({
    title: "外部のタイトル",
    description: "",
    image: null,
    siteName: "Example",
    icon: null,
  });
});

afterEach(() => {
  resetSeenEncryptedChannels();
  for (const key of Object.keys(localStorage)) {
    if (key.startsWith(ENCRYPTED_CHANNEL_STORAGE_PREFIX)) localStorage.removeItem(key);
  }
  localStorage.removeItem(LOCALE_STORAGE_KEY);
  vi.restoreAllMocks();
});

describe("メッセージのリンク", () => {
  it("URL を「○○さん: #チャンネル名」に短くし、本文を埋め込みで見せる", async () => {
    setup({ content: link("c-general", "?message=m-general") });

    // 本文中のリンクが短くなる（URL そのものは出さない）。
    const inline = await screen.findByRole("link", { name: "アメシュさん: #general" });
    expect(inline.getAttribute("href")).toBe("/channels/c-general?message=m-general");
    expect(inline.textContent).not.toContain("http");

    // 埋め込みに、投稿者・チャンネル・本文が出る。
    const embed = await screen.findByRole("link", { name: /アメシュ.*#general.*テスト/ });
    expect(embed).not.toBe(inline);
    expect(within(embed).getByText("アメシュ")).toBeInTheDocument();
    expect(within(embed).getByText("#general")).toBeInTheDocument();
    expect(within(embed).getByText("テスト")).toBeInTheDocument();
    expect(embed.querySelector("time")?.getAttribute("datetime")).toBe("2026-11-23T07:30:00.000Z");
    expect(embed.getAttribute("target")).toBeNull();
  });

  it("埋め込みを押しても、同じタブでそのメッセージへ移動する", async () => {
    const { router } = setup({ content: link("c-general", "?message=m-general") });
    const embed = await screen.findByRole("link", { name: /アメシュ.*テスト/ });
    await userEvent.click(embed);
    await waitFor(() =>
      expect(screen.getByLabelText("Opened").textContent).toBe("c-general|m-general|"),
    );
    expect(router.state.location.href).toBe("/channels/c-general?message=m-general");
  });

  it("短くしたリンクを押しても、同じタブで移動する", async () => {
    setup({ content: link("c-general", "?message=m-general") });
    await userEvent.click(await screen.findByRole("link", { name: "アメシュさん: #general" }));
    await waitFor(() =>
      expect(screen.getByLabelText("Opened").textContent).toBe("c-general|m-general|"),
    );
  });

  it("スレッドの返信は「内のスレッド」と示し、スレッドごと開く", async () => {
    setup({ content: link("c-general", "?thread=m-general&message=m-reply") });
    const embed = await screen.findByRole("link", { name: /返信です/ });
    expect(within(embed).getByText("#general 内のスレッド")).toBeInTheDocument();
    await userEvent.click(embed);
    await waitFor(() =>
      expect(screen.getByLabelText("Opened").textContent).toBe("c-general|m-reply|m-general"),
    );
  });

  it("本文の装飾とリンクは外した文字で見せ、埋め込みの中にリンクを入れ子にしない", async () => {
    setup({ content: link("c-general", "?message=m-md") });
    const embed = await screen.findByRole("link", { name: /太字とリンクとcode/ });
    expect(embed.querySelector("a")).toBeNull();
    expect(embed.querySelector("strong, b, code")).toBeNull();
  });

  it("長い本文は行数で切り、カードの幅を超えない", async () => {
    setup({ content: link("c-general", "?message=m-long") });
    const embed = await screen.findByRole("link", { name: /とても長い/ });
    const body = within(embed).getByText(/とても長い/);
    expect(getComputedStyle(body).webkitLineClamp).toBe("3");
    expect(embed.getBoundingClientRect().width).toBeLessThanOrEqual(448);
    expect(embed.scrollWidth).toBeLessThanOrEqual(embed.clientWidth);
  });

  it("本文がなく添付だけのメッセージは、添付の数を出す", async () => {
    setup({ content: link("c-general", "?message=m-files") });
    expect(await screen.findByText("添付ファイル 2 件")).toBeInTheDocument();
  });

  it("コメントのない転送の投稿は、空のカードにせず、転送したことを書く", async () => {
    setup({ content: link("c-general", "?message=m-forward-only") });
    const embed = await screen.findByRole("link", { name: /メッセージを転送しました/ });
    expect(within(embed).getByText("アメシュ")).toBeInTheDocument();
    // 転送元の本文は、埋め込みに展開しない（`m-general` の本文）。
    expect(within(embed).queryByText("テスト")).toBeNull();
  });

  it("コメントのある転送の投稿は、コメントを出す", async () => {
    setup({ content: link("c-general", "?message=m-forward-comment") });
    const embed = await screen.findByRole("link", { name: /これを見てください/ });
    expect(within(embed).queryByText("メッセージを転送しました")).toBeNull();
  });

  it("DM は内部の名前ではなく、総称で示す", async () => {
    setup({ content: link("c-dm", "?message=m-dm") });
    expect(
      await screen.findByRole("link", { name: "アメシュさん: ダイレクトメッセージ" }),
    ).toBeInTheDocument();
    const embed = await screen.findByRole("link", { name: /DM の内容/ });
    expect(embed.textContent).not.toContain("dm:u-alice");
  });

  it("非公開チャンネルでも、参加していて見られるなら表示する", async () => {
    setup({ content: link("c-secret", "?message=m-secret") });
    expect(await screen.findByRole("link", { name: /参加者だけの話/ })).toBeInTheDocument();
    expect(await screen.findByRole("link", { name: "アメシュさん: #secret" })).toBeInTheDocument();
  });
});

describe("暗号化チャンネルのメッセージ", () => {
  it("本文は出さず、暗号化されていることを書く", async () => {
    setup({ content: link("c-sealed", "?message=m-sealed") });
    const embed = await screen.findByRole("link", { name: /このメッセージは暗号化されています/ });
    expect(within(embed).getByText("このメッセージは暗号化されています")).toBeInTheDocument();
    expect(document.body.textContent).not.toContain("暗号化された本文は出さない");
    // 投稿者とチャンネルは本文ではないので出す。押せば移動できる。
    expect(within(embed).getByText("アメシュ")).toBeInTheDocument();
    expect(within(embed).getByText("#sealed")).toBeInTheDocument();
    await userEvent.click(embed);
    await waitFor(() =>
      expect(screen.getByLabelText("Opened").textContent).toBe("c-sealed|m-sealed|"),
    );
  });
});

describe("暗号化の格下げ対策", () => {
  // サーバーが `encrypted = false` を配っても、暗号化として扱うもの（docs/e2ee.md の「暗号化の格下げを防ぐ」）。
  const hiddenText = (text: string) => expect(document.body.textContent).not.toContain(text);

  it("暗号文のある行は、チャンネルが平文でも、本文を出さずに暗号化と書く", async () => {
    setup({ content: link("c-downgraded", "?message=m-cipher") });
    const embed = await screen.findByRole("link", { name: /このメッセージは暗号化されています/ });
    expect(within(embed).getByText("アメシュ")).toBeInTheDocument();
  });

  it("同じチャンネルの別の行に暗号文があれば、平文の行も、本文を出さずに暗号化と書く", async () => {
    // 対象の行は平文（サーバーが差し込んだ行かもしれない）。読み込み済みの行に暗号文があるので、
    // `MessagePane` と同じく、チャンネルを格下げされた暗号化チャンネルとして扱う。
    setup({ content: link("c-downgraded", "?message=m-plain-sibling") });
    expect(
      await screen.findByRole("link", { name: /このメッセージは暗号化されています/ }),
    ).toBeInTheDocument();
    hiddenText("暗号文の隣の平文");
  });

  it("チャンネルだけを指すリンクは、暗号文の有無を見ない（そのチャンネルの messages を同期しないため）", async () => {
    // 名前と説明だけを出すカードなので、本文の漏れはない。記録・`edm:` による判定は、メッセージのリンクと同じ。
    const { messagesFor } = setup({ content: link("c-downgraded") });
    expect(await screen.findAllByRole("link", { name: /#downgraded/ })).toHaveLength(2);
    await new Promise((resolve) => setTimeout(resolve, 100));
    expect(screen.queryByText("暗号化チャンネル")).toBeNull();
    expect(messagesFor).not.toHaveBeenCalled();
  });

  it("暗号文のないチャンネルの行は、本文を出す", async () => {
    setup({ content: link("c-general", "?message=m-general") });
    expect(await screen.findByRole("link", { name: /アメシュ.*テスト/ })).toBeInTheDocument();
    expect(screen.queryByText("このメッセージは暗号化されています")).toBeNull();
  });

  it("名前が `edm:` のチャンネルは、暗号化として扱う", async () => {
    setup({ content: link("c-edm", "?message=m-edm"), visible: [edm] });
    expect(
      await screen.findByRole("link", { name: /このメッセージは暗号化されています/ }),
    ).toBeInTheDocument();
    hiddenText("edm の平文");
  });

  it("以前に暗号化チャンネルとして見たチャンネルは、暗号化として扱う", async () => {
    noteEncryptedChannel("c-recorded");
    setup({ content: link("c-recorded", "?message=m-recorded"), visible: [recorded] });
    expect(
      await screen.findByRole("link", { name: /このメッセージは暗号化されています/ }),
    ).toBeInTheDocument();
    hiddenText("記録済みの平文");
  });

  it("一覧で暗号化と見たチャンネルは、あとで平文として配られても、暗号化として扱う", async () => {
    const encrypted = { ...recorded, encrypted: true };
    const { updateDirectory } = setup({
      content: link("c-recorded", "?message=m-recorded"),
      visible: [encrypted],
    });
    expect(
      await screen.findByRole("link", { name: /このメッセージは暗号化されています/ }),
    ).toBeInTheDocument();

    // サーバーが `encrypted = false` を配る（格下げ）。
    await updateDirectory({ visible: [{ ...encrypted, encrypted: false }], ready: true });

    await waitFor(() =>
      expect(screen.getByText("このメッセージは暗号化されています")).toBeInTheDocument(),
    );
    hiddenText("記録済みの平文");
  });

  it("チャンネルのリンクも、格下げされたチャンネルを暗号化チャンネルと示す", async () => {
    noteEncryptedChannel("c-recorded");
    setup({ content: link("c-recorded"), visible: [recorded] });
    expect(await screen.findByText("暗号化チャンネル")).toBeInTheDocument();
  });
});

describe("暗号ストアの記録による格下げ対策", () => {
  // `localStorage` の記録がない端末で、サーバーが `encrypted = false` を配り、平文の行が届いた場合。
  // 暗号ストアの記録（`megolmIsKnownEncrypted`）で、暗号化チャンネルと分かる。
  const target = link("c-recorded", "?message=m-recorded");
  const options = { content: target, visible: [recorded] };

  it("暗号ストアの記録で暗号化と分かるチャンネルは、本文を出さずに暗号化と書く", async () => {
    const worker = workerKnowing(async (id) => id === "c-recorded");
    setup({ ...options, worker });
    expect(
      await screen.findByRole("link", { name: /このメッセージは暗号化されています/ }),
    ).toBeInTheDocument();
    expect(document.body.textContent).not.toContain("記録済みの平文");
    expect(worker).toHaveBeenCalledWith({ op: "megolmIsKnownEncrypted", channelId: "c-recorded" });
  });

  it("暗号ストアに記録がなければ、本文を出す", async () => {
    setup({ ...options, worker: workerKnowing(async () => false) });
    expect(await screen.findByRole("link", { name: /記録済みの平文/ })).toBeInTheDocument();
    expect(screen.queryByText("このメッセージは暗号化されています")).toBeNull();
  });

  it("確認が終わるまで、平文の本文を出さず、読み込み中の枠を置く", async () => {
    // 本文中の短い表示とカードが、それぞれ問い合わせる。どちらにも、あとで答える。
    const answers: ((known: boolean) => void)[] = [];
    const worker = workerKnowing(() => new Promise<boolean>((resolve) => answers.push(resolve)));
    setup({ ...options, worker });
    expect(await screen.findByRole("status")).toHaveTextContent("リンク先を読み込み中");
    await new Promise((resolve) => setTimeout(resolve, 200));
    expect(document.body.textContent).not.toContain("記録済みの平文");
    // 本文中のリンクも、確認が終わるまで URL のまま。
    expect(screen.getByRole("link").textContent).toContain("/channels/c-recorded");

    await act(async () => {
      for (const answer of answers) answer(false);
    });
    expect(await screen.findByRole("link", { name: /記録済みの平文/ })).toBeInTheDocument();
    // 問い合わせは、短い表示とカードの 2 回だけ。描き直しのたびに繰り返さない。
    await new Promise((resolve) => setTimeout(resolve, 100));
    expect(worker).toHaveBeenCalledTimes(2);
  });

  it("確認できなかった（問い合わせが失敗した）ときは、平文の本文を出さない", async () => {
    const worker = workerKnowing(() => Promise.reject(new Error("store unavailable")));
    setup({ ...options, worker });
    await waitFor(() => expect(worker).toHaveBeenCalled());
    await new Promise((resolve) => setTimeout(resolve, 200));
    expect(document.body.textContent).not.toContain("記録済みの平文");
    expect(screen.getByRole("status")).toBeInTheDocument();
  });

  it("本文を出さないと決まっている行は、確認を待たずに暗号化と出す", async () => {
    setup({
      content: link("c-downgraded", "?message=m-cipher"),
      worker: workerKnowing(() => new Promise<boolean>(() => {})),
    });
    expect(
      await screen.findByRole("link", { name: /このメッセージは暗号化されています/ }),
    ).toBeInTheDocument();
  });

  it("サーバーが暗号化と配るチャンネルは、暗号ストアにも覚えさせる", async () => {
    const worker = workerKnowing(async () => false);
    setup({ content: link("c-sealed", "?message=m-sealed"), worker });
    await screen.findByRole("link", { name: /このメッセージは暗号化されています/ });
    await waitFor(() =>
      expect(worker).toHaveBeenCalledWith({
        op: "megolmRememberEncrypted",
        channelId: "c-sealed",
      }),
    );
  });

  it("チャンネルだけを指すリンクも、暗号ストアの記録で暗号化チャンネルと示す", async () => {
    setup({
      content: link("c-recorded"),
      visible: [recorded],
      worker: workerKnowing(async () => true),
    });
    expect(await screen.findByText("暗号化チャンネル")).toBeInTheDocument();
  });

  it("一覧にない（見られない）チャンネルは、暗号ストアに問い合わせない", async () => {
    const worker = workerKnowing(async () => true);
    setup({ content: link("c-hidden", "?message=m-hidden"), worker });
    expect(await screen.findByText("メッセージを表示できません")).toBeInTheDocument();
    await new Promise((resolve) => setTimeout(resolve, 100));
    expect(worker).not.toHaveBeenCalledWith(expect.objectContaining({ channelId: "c-hidden" }));
  });
});

describe("暗号ストアを開く前の扱い", () => {
  const options = { content: link("c-recorded", "?message=m-recorded"), visible: [recorded] };
  /** `E2eeProvider` が、暗号ストアを開けていない理由を添えて配る値。 */
  const notReady = (kind: string) =>
    ({ worker: null, notReadyReason: { kind } }) as unknown as E2eeContextValue;

  it("開いているところ（まもなく確かめられる）は、平文の本文を出さず、枠を置く", async () => {
    setup({ ...options, e2eeValue: notReady("starting") });
    expect(await screen.findByRole("status")).toHaveTextContent("リンク先を読み込み中");
    await new Promise((resolve) => setTimeout(resolve, 200));
    expect(document.body.textContent).not.toContain("記録済みの平文");
  });

  it("開き終わるまで本文を出さず、記録があれば、一度も本文を出さずに暗号化と書く", async () => {
    // 本文が一瞬でも出ないことを、DOM の変化を見張って確かめる。
    let leaked = false;
    const observer = new MutationObserver(() => {
      if (document.body.textContent?.includes("記録済みの平文")) leaked = true;
    });
    observer.observe(document.body, { childList: true, subtree: true, characterData: true });
    try {
      const { updateCryptoStore } = setup({ ...options, e2eeValue: notReady("starting") });
      await screen.findByRole("status");
      await new Promise((resolve) => setTimeout(resolve, 100));
      await updateCryptoStore(cryptoStore(workerKnowing(async () => true)));
      expect(
        await screen.findByRole("link", { name: /このメッセージは暗号化されています/ }),
      ).toBeInTheDocument();
      expect(leaked).toBe(false);
    } finally {
      observer.disconnect();
    }
  });

  it("開き終えて記録がなければ、本文を出す", async () => {
    const { updateCryptoStore } = setup({ ...options, e2eeValue: notReady("starting") });
    await screen.findByRole("status");
    await updateCryptoStore(cryptoStore(workerKnowing(async () => false)));
    expect(await screen.findByRole("link", { name: /記録済みの平文/ })).toBeInTheDocument();
  });

  it.each([
    "unsupported",
    "otherTab",
    "offline",
    "failed",
    "deviceLimit",
    "signedOut",
    "userUnavailable",
  ])("このタブで開けない状態（%s）は、localStorage などの判定だけで表示する", async (kind) => {
    // 暗号ストアの答えを待っても得られないので、リンクの表示を止めない（`MessagePane` と同じ）。
    setup({ ...options, e2eeValue: notReady(kind) });
    expect(await screen.findByRole("link", { name: /記録済みの平文/ })).toBeInTheDocument();
  });

  it("このタブで開けない状態でも、localStorage の記録があれば、本文を出さない", async () => {
    noteEncryptedChannel("c-recorded");
    setup({ ...options, e2eeValue: notReady("otherTab") });
    expect(
      await screen.findByRole("link", { name: /このメッセージは暗号化されています/ }),
    ).toBeInTheDocument();
    expect(document.body.textContent).not.toContain("記録済みの平文");
  });
});

describe("連携カード・部品の投稿", () => {
  it("連携カードの投稿は、代わりの本文を出さず、App のメッセージと書く", async () => {
    setup({ content: link("c-general", "?message=m-card") });
    const embed = await screen.findByRole("link", { name: /Appのメッセージ/ });
    expect(within(embed).getByText("アメシュ")).toBeInTheDocument();
    expect(document.body.textContent).not.toContain("カードの代わりの本文");
  });

  it("本文を隠す部品の投稿も、代わりの本文を出さない", async () => {
    setup({ content: link("c-general", "?message=m-parts") });
    expect(await screen.findByRole("link", { name: /Appのメッセージ/ })).toBeInTheDocument();
    expect(document.body.textContent).not.toContain("部品の代わりの本文");
  });
});

describe("見られないリンク先", () => {
  it("一覧にない（非公開で未参加の）チャンネルは「表示できません」にし、チャンネル名も出さない", async () => {
    const { messagesFor } = setup({
      content: link("c-hidden", "?message=m-hidden"),
      hiddenRows: [
        message({ id: "m-hidden", channel_id: "c-hidden", content: "見えてはいけない" }),
      ],
    });
    expect(await screen.findByText("メッセージを表示できません")).toBeInTheDocument();
    // 埋め込みは、押して移動するリンクではない。
    expect(screen.getAllByRole("link")).toHaveLength(1);
    // 短くせず、URL のまま。名前も本文も出さない。
    expect(screen.getByRole("link").textContent).toContain("/channels/c-hidden?message=m-hidden");
    expect(document.body.textContent).not.toContain("秘密の計画");
    expect(document.body.textContent).not.toContain("見えてはいけない");
    // 見られないチャンネルのメッセージは、問い合わせもしない。
    expect(messagesFor).not.toHaveBeenCalledWith("c-hidden");
  });

  it("存在しないチャンネルも、非公開のものと同じ表示にする", async () => {
    setup({ content: link("c-nowhere", "?message=m-1") });
    expect(await screen.findByText("メッセージを表示できません")).toBeInTheDocument();
  });

  it("チャンネルのリンクは「チャンネルを表示できません」にする", async () => {
    setup({ content: link("c-hidden") });
    expect(await screen.findByText("チャンネルを表示できません")).toBeInTheDocument();
    expect(screen.queryByText("メッセージを表示できません")).toBeNull();
  });

  it("削除済みのメッセージは「表示できません」にし、本文は出さない", async () => {
    setup({ content: link("c-general", "?message=m-deleted") });
    expect(await screen.findByText("メッセージを表示できません")).toBeInTheDocument();
    expect(document.body.textContent).not.toContain("消えた本文");
    // チャンネルは見られるが、消えたメッセージの短い表示は作らない。
    expect(screen.getAllByRole("link")).toHaveLength(1);
    expect(screen.getByRole("link").textContent).toContain("/channels/c-general?message=m-deleted");
  });

  it("チャンネルは見られても、メッセージが見つからなければ「表示できません」にする", async () => {
    setup({ content: link("c-general", "?message=m-nothing") });
    expect(await screen.findByText("メッセージを表示できません")).toBeInTheDocument();
  });
});

describe("一覧の同期を待つ間", () => {
  it("一覧の同期が終わるまでは、「表示できません」と決めない", async () => {
    setup({ content: link("c-hidden", "?message=m-hidden"), ready: false });
    // 読み込み中の枠だけが出て、見られないという表示は出さない。
    await new Promise((resolve) => setTimeout(resolve, 100));
    expect(screen.queryByText("メッセージを表示できません")).toBeNull();
  });

  it("同期が終わって一覧に現れたチャンネルは、取得を始めて、表示に切り替わる", async () => {
    const { messagesFor, updateDirectory } = setup({
      content: link("c-general", "?message=m-general"),
      visible: [],
      ready: false,
    });
    await new Promise((resolve) => setTimeout(resolve, 50));
    // 同期前は、取得せず、「表示できません」とも決めない。
    expect(messagesFor).not.toHaveBeenCalled();
    expect(screen.queryByText("メッセージを表示できません")).toBeNull();
    expect(screen.queryByText("テスト")).toBeNull();

    await updateDirectory({ visible: [general], ready: true });

    // 同じチャンネルのメッセージを取得し、短い表示と埋め込みが出る。
    expect(await screen.findByRole("link", { name: /アメシュ.*テスト/ })).toBeInTheDocument();
    expect(await screen.findByRole("link", { name: "アメシュさん: #general" })).toBeInTheDocument();
    expect(messagesFor).toHaveBeenCalledWith("c-general");
    expect(screen.queryByText("メッセージを表示できません")).toBeNull();
  });

  it("同期が終わっても一覧になかったチャンネルは、「表示できません」に切り替わる", async () => {
    const { messagesFor, updateDirectory } = setup({
      content: link("c-hidden", "?message=m-hidden"),
      visible: [],
      ready: false,
    });
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(screen.queryByText("メッセージを表示できません")).toBeNull();

    await updateDirectory({ visible: [general], ready: true });

    expect(await screen.findByText("メッセージを表示できません")).toBeInTheDocument();
    expect(messagesFor).not.toHaveBeenCalledWith("c-hidden");
  });
});

describe("チャンネルのリンク", () => {
  it("「#チャンネル名」に短くし、チャンネルの説明を埋め込みで見せる", async () => {
    setup({ content: link("c-general") });
    expect(await screen.findByRole("link", { name: "#general" })).toBeInTheDocument();
    const embed = await screen.findByRole("link", { name: /全体の連絡/ });
    expect(within(embed).getByText("#general")).toBeInTheDocument();
    await userEvent.click(embed);
    await waitFor(() => expect(screen.getByLabelText("Opened").textContent).toBe("c-general||"));
  });

  it("非公開・暗号化のチャンネルは、その旨を書く", async () => {
    setup({ content: `${link("c-secret")} ${link("c-sealed")}` });
    expect(await screen.findByText("非公開チャンネル")).toBeInTheDocument();
    expect(screen.getByText("暗号化チャンネル")).toBeInTheDocument();
  });
});

describe("短くする範囲", () => {
  it("文字を書いたリンク（[文字](URL)）は、書いた文字を変えない。埋め込みは出す", async () => {
    setup({ content: `[議論のスレッド](${link("c-general", "?message=m-general")})` });
    expect(await screen.findByRole("link", { name: "議論のスレッド" })).toBeInTheDocument();
    expect(await screen.findByRole("link", { name: /アメシュ.*テスト/ })).toBeInTheDocument();
    expect(screen.queryByRole("link", { name: "アメシュさん: #general" })).toBeNull();
  });

  it("検索語を強調している間は、強調する URL の文字を短くしない", async () => {
    setup({
      content: link("c-general", "?message=m-general"),
      body: "content",
      highlight: ["general"],
    });
    const anchor = await screen.findByRole("link");
    expect(anchor.textContent).toContain("/channels/c-general?message=m-general");
    expect(anchor.querySelector("mark")).not.toBeNull();
  });

  it("別の origin の URL は、短くも埋め込みにもしない", async () => {
    setup({ content: "https://example.com/channels/c-general?message=m-general" });
    const anchor = await screen.findByRole("link", { name: /example.com/ });
    expect(anchor.getAttribute("target")).toBe("_blank");
    expect(screen.queryByText(/アメシュ/)).toBeNull();
  });

  it("チャンネルの一覧がない画面では、今までどおり URL のまま出す", async () => {
    setup({ content: link("c-general", "?message=m-general"), directory: false });
    const anchors = await screen.findAllByRole("link");
    expect(anchors.length).toBeGreaterThan(0);
    for (const anchor of anchors) expect(anchor.textContent).not.toContain("アメシュさん");
    expect(screen.queryByText("メッセージを表示できません")).toBeNull();
  });
});

describe("外部へ送らない", () => {
  it("アプリ内のリンクは Summaly に送らず、外部の URL だけを送る", async () => {
    setup({
      content: `${link("c-general", "?message=m-general")}\nhttps://example.com/page`,
    });
    expect(await screen.findByRole("link", { name: /アメシュ.*テスト/ })).toBeInTheDocument();
    await screen.findByRole("link", { name: /外部のタイトル/ });
    const requested = summaly.get.mock.calls.map(([url]) => String(url));
    expect(requested).toEqual(["https://example.com/page"]);
  });

  it("アプリ内のリンクだけなら、Summaly の設定の取得も始めない", async () => {
    setup({ content: link("c-general", "?message=m-general") });
    await screen.findByRole("link", { name: /アメシュ.*テスト/ });
    expect(summaly.get).not.toHaveBeenCalled();
  });

  // 外部のカードは、取得結果が URL ごとにモジュール内で共有される。ほかのテストと URL を分け、取得の有無を確かめる。
  it("暗号化チャンネルの投稿でも、アプリ内のリンクはすぐ埋め込み、外部の URL だけを押したときに取得する", async () => {
    setup({
      content: `${link("c-general", "?message=m-general")}\nhttps://example.com/manual-page`,
      previews: "manual",
    });
    expect(await screen.findByRole("link", { name: /アメシュ.*テスト/ })).toBeInTheDocument();
    const button = screen.getByRole("button", { name: "リンクのプレビューを読み込む" });
    expect(summaly.get).not.toHaveBeenCalled();
    await userEvent.click(button);
    await screen.findByRole("link", { name: /外部のタイトル/ });
    expect(summaly.get.mock.calls.map(([url]) => String(url))).toEqual([
      "https://example.com/manual-page",
    ]);
  });

  it("暗号化チャンネルの投稿に、アプリ内のリンクしかなければ、読み込みの操作を出さない", async () => {
    setup({ content: link("c-general", "?message=m-general"), previews: "manual" });
    await screen.findByRole("link", { name: /アメシュ.*テスト/ });
    expect(screen.queryByRole("button", { name: "リンクのプレビューを読み込む" })).toBeNull();
  });
});

describe("転送（引用）のカードの中のリンク", () => {
  const entry = (content: string): ForwardEntryState => ({
    status: "ok",
    id: "m-forwarded",
    channel_id: "c-general",
    channel_name: "general",
    is_private: false,
    is_dm: false,
    parent_id: null,
    user_id: "u-alice",
    created_at: ts(45),
    updated_at: null,
    content,
    card: null,
    components: null,
    message_kind: "message",
    author: null,
    emojis: {},
    attachments: [],
    has_forwards: false,
  });
  const forwardCard = (compact: boolean) => (
    <ForwardedMessageCard
      messageId="m-forwarded"
      entry={entry(`転送元の本文 ${link("c-general", "?message=m-general")}`)}
      compact={compact}
    />
  );

  it("引用の中のメッセージリンクは、短くして埋め込み、リンクを入れ子にしない", async () => {
    setup({ content: "", node: forwardCard(false) });
    const quote = await screen.findByRole("blockquote");
    expect(
      await within(quote).findByRole("link", { name: "アメシュさん: #general" }),
    ).toBeInTheDocument();
    expect(
      await within(quote).findByRole("link", { name: /アメシュ.*#general.*テスト/ }),
    ).toBeInTheDocument();
    expect(quote.querySelector("a a")).toBeNull();
  });

  it("転送ダイアログの簡潔なプレビューは、埋め込みを出さない", async () => {
    setup({ content: "", node: forwardCard(true) });
    const quote = await screen.findByRole("blockquote");
    // 本文中の短い表示は出る。埋め込み（本文を含むカード）は出さない。
    await within(quote).findByRole("link", { name: "アメシュさん: #general" });
    expect(within(quote).queryByRole("link", { name: /アメシュ.*#general.*テスト/ })).toBeNull();
    expect(within(quote).queryByRole("status")).toBeNull();
  });
});

describe("操作", () => {
  it.each([
    ["Ctrl", { ctrlKey: true }],
    ["Meta", { metaKey: true }],
    ["Shift", { shiftKey: true }],
  ])(
    "埋め込みを %s を押しながら押しても、この画面は動かさない（別のタブはブラウザーに任せる）",
    async (_name, keys) => {
      const { router } = setup({ content: link("c-general", "?message=m-general") });
      const embed = await screen.findByRole("link", { name: /アメシュ.*テスト/ });
      const prevented: boolean[] = [];
      const observe = (event: MouseEvent) => {
        prevented.push(event.defaultPrevented);
        event.preventDefault();
      };
      document.addEventListener("click", observe);
      try {
        fireEvent.click(embed, { button: 0, ...keys });
      } finally {
        document.removeEventListener("click", observe);
      }
      expect(prevented).toEqual([false]);
      expect(router.state.location.href).toBe("/");
    },
  );

  it("キーボードで埋め込みへ移り、Enter で移動できる", async () => {
    setup({ content: link("c-general", "?message=m-general") });
    const embed = await screen.findByRole("link", { name: /アメシュ.*テスト/ });
    await act(async () => embed.focus());
    expect(document.activeElement).toBe(embed);
    await userEvent.keyboard("{Enter}");
    await waitFor(() =>
      expect(screen.getByLabelText("Opened").textContent).toBe("c-general|m-general|"),
    );
  });
});

describe("画面の近くに来るまで取得しない", () => {
  /** 画面から遠い位置（スクロール枠の 3000px 下）に、`children` を置く。 */
  async function renderFar(
    children: ReactNode,
    visible: ChannelRow[] = [general],
    worker?: WorkerRequest,
  ) {
    const e2ee = cryptoStore(worker ?? workerKnowing(async () => false));
    const bundle = createStoryCollections({
      currentUserId: "u-alice",
      users,
      channels: [general, secretMember],
      messages,
    });
    const messagesFor = vi.fn(bundle.messages);
    bundle.messages = messagesFor as typeof bundle.messages;
    const root = createRootRoute({ component: Outlet });
    const home = createRoute({
      getParentRoute: () => root,
      path: "/",
      component: () => (
        <E2eeContext.Provider value={e2ee}>
          <LocaleProvider>
            <CollectionsProvider value={bundle}>
              <MfmChannelsProvider channels={visible}>
                <div data-testid="pane" style={{ height: 200, overflowY: "auto" }}>
                  <div style={{ height: 3000 }} />
                  {children}
                  <div style={{ height: 3000 }} />
                </div>
              </MfmChannelsProvider>
            </CollectionsProvider>
          </LocaleProvider>
        </E2eeContext.Provider>
      ),
    });
    const router = createRouter({
      routeTree: root.addChildren([home]),
      history: createMemoryHistory({ initialEntries: ["/"] }),
    });
    render(<RouterProvider router={router} />);
    const pane = await screen.findByTestId("pane");
    await new Promise((resolve) => setTimeout(resolve, 150));
    return { pane, messagesFor };
  }

  it("遅延表示（defer）のカードは、画面の近くに来てから、メッセージを取得する", async () => {
    const { pane, messagesFor } = await renderFar(
      <LinkPreviews defer content={link("c-general", "?message=m-general")} />,
    );
    // 画面から遠い間は、取得せず、読み込み中の枠だけを置く。
    expect(messagesFor).not.toHaveBeenCalled();
    expect(screen.queryByText(/アメシュ/)).toBeNull();
    expect(screen.getByRole("status")).toHaveTextContent("リンク先を読み込み中");

    pane.scrollTop = 3000;
    expect(await screen.findByRole("link", { name: /アメシュ.*テスト/ })).toBeInTheDocument();
    expect(messagesFor).toHaveBeenCalledWith("c-general");
  });

  it("本文中の短い表示も、画面の近くに来てから、メッセージを取得する", async () => {
    const { pane, messagesFor } = await renderFar(
      <MfmContent content={link("c-general", "?message=m-general")} />,
    );
    // 遠い間は、取得せず、URL のまま。
    expect(messagesFor).not.toHaveBeenCalled();
    expect(screen.getByRole("link").textContent).toContain("/channels/c-general?message=m-general");

    pane.scrollTop = 3000;
    expect(await screen.findByRole("link", { name: "アメシュさん: #general" })).toBeInTheDocument();
    expect(messagesFor).toHaveBeenCalledWith("c-general");
  });

  it("一度表示したカードは、画面から遠ざかっても、枠に戻らない", async () => {
    const { pane } = await renderFar(
      <LinkPreviews defer content={link("c-general", "?message=m-general")} />,
    );
    pane.scrollTop = 3000;
    expect(await screen.findByRole("link", { name: /アメシュ.*テスト/ })).toBeInTheDocument();

    pane.scrollTop = 0;
    await new Promise((resolve) => setTimeout(resolve, 300));
    expect(screen.getByRole("link", { name: /アメシュ.*テスト/ })).toBeInTheDocument();
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("同じ要素が別のリンクに使い回されたら、新しいリンク先は、近くに来るまで取得しない", async () => {
    let setContent: (content: string) => void = () => {};
    function Switchable() {
      const [content, set] = useState(link("c-general", "?message=m-general"));
      setContent = set;
      return <MfmContent content={content} />;
    }
    const { pane, messagesFor } = await renderFar(<Switchable />, [general, secretMember]);
    pane.scrollTop = 3000;
    expect(await screen.findByRole("link", { name: "アメシュさん: #general" })).toBeInTheDocument();

    // 画面から遠ざかったまま、同じ要素が別のチャンネルのリンクになる（本文の編集など）。
    pane.scrollTop = 0;
    await new Promise((resolve) => setTimeout(resolve, 300));
    await act(async () => setContent(link("c-secret", "?message=m-secret")));
    await new Promise((resolve) => setTimeout(resolve, 300));
    // 新しいリンク先は、取得せず、URL のまま。
    expect(messagesFor).not.toHaveBeenCalledWith("c-secret");
    expect(screen.getByRole("link").textContent).toContain("/channels/c-secret?message=m-secret");

    // 近くに来たら取得する。
    pane.scrollTop = 3000;
    expect(await screen.findByRole("link", { name: "アメシュさん: #secret" })).toBeInTheDocument();
    expect(messagesFor).toHaveBeenCalledWith("c-secret");

    // 取得済みのリンク先に戻ったら、遠くにいても、取得済みの表示のまま。
    pane.scrollTop = 0;
    await new Promise((resolve) => setTimeout(resolve, 300));
    await act(async () => setContent(link("c-general", "?message=m-general")));
    expect(await screen.findByRole("link", { name: "アメシュさん: #general" })).toBeInTheDocument();
  });

  // チャットの投稿の本文。スレッドやキャッチアップは、画面外の行もマウントしたままにする。
  it("投稿の本文の埋め込みと外部のカードも、画面の近くに来てから取得する", async () => {
    const worker = workerKnowing(async () => false);
    const { pane, messagesFor } = await renderFar(
      <MessageBody
        content={`${link("c-general", "?message=m-general")}\nhttps://example.com/far-page`}
      />,
      [general],
      worker,
    );
    expect(messagesFor).not.toHaveBeenCalled();
    expect(worker).not.toHaveBeenCalled();
    expect(summaly.get).not.toHaveBeenCalled();
    expect(screen.getByRole("status")).toHaveTextContent("リンク先を読み込み中");

    pane.scrollTop = 3000;
    expect(await screen.findByRole("link", { name: /アメシュ.*テスト/ })).toBeInTheDocument();
    expect(await screen.findByRole("link", { name: /外部のタイトル/ })).toBeInTheDocument();
    expect(messagesFor).toHaveBeenCalledWith("c-general");
    expect(summaly.get.mock.calls.map(([url]) => String(url))).toEqual([
      "https://example.com/far-page",
    ]);
  });

  it("画面から遠いメッセージのリンクは、暗号ストアにも問い合わせない", async () => {
    const worker = workerKnowing(async () => false);
    const { pane } = await renderFar(
      <MfmContent content={link("c-general", "?message=m-general")} />,
      [general],
      worker,
    );
    expect(worker).not.toHaveBeenCalled();

    pane.scrollTop = 3000;
    expect(await screen.findByRole("link", { name: "アメシュさん: #general" })).toBeInTheDocument();
    expect(worker).toHaveBeenCalledWith({ op: "megolmIsKnownEncrypted", channelId: "c-general" });
  });

  it("一度表示した短い表示は、画面から遠ざかっても、URL に戻らない", async () => {
    const { pane } = await renderFar(
      <MfmContent content={link("c-general", "?message=m-general")} />,
    );
    pane.scrollTop = 3000;
    expect(await screen.findByRole("link", { name: "アメシュさん: #general" })).toBeInTheDocument();

    pane.scrollTop = 0;
    await new Promise((resolve) => setTimeout(resolve, 300));
    expect(screen.getByRole("link", { name: "アメシュさん: #general" })).toBeInTheDocument();
  });
});
