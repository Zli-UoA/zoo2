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
import { afterEach, beforeEach, describe, expect, it, vi } from "vite-plus/test";

import { createStoryCollections } from "../../../../.storybook/fixtures";
import { LOCALE_STORAGE_KEY } from "../../../lib/i18n/locale";
import { CollectionsProvider } from "../../../lib/electric/collections-context";
import type { ChannelRow, MessageRow, UserRow } from "../../../lib/electric/types";
import { LocaleProvider } from "../../i18n/LocaleProvider";
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

const messages: MessageRow[] = [
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

interface SetupOptions {
  content: string;
  /** メッセージ取得の対象にできるチャンネル（ユーザーが見られるもの）。 */
  visible?: ChannelRow[];
  ready?: boolean;
  /** 一覧には出ないが、コレクションには行がある（サーバーの権限で守られる）チャンネル。 */
  hiddenRows?: MessageRow[];
  previews?: "auto" | "manual";
  /** `false` で、チャンネルの一覧（Provider）を置かない（公開プロフィールなどを想定）。 */
  directory?: boolean;
  highlight?: string[];
  body?: "message" | "content";
}

function setup({
  content,
  visible = [general, secretMember, sealed, dm],
  ready = true,
  hiddenRows = [],
  previews = "auto",
  directory = true,
  highlight,
  body = "message",
}: SetupOptions) {
  const bundle = createStoryCollections({
    currentUserId: "u-alice",
    users,
    channels: [
      ...visible,
      channel({ id: "c-hidden", channel_name: "秘密の計画", is_private: true }),
    ],
    messages: [...messages, ...hiddenRows],
  });
  const original = bundle.messages;
  const messagesFor = vi.fn((channelId: string) => original(channelId));
  bundle.messages = messagesFor as typeof bundle.messages;

  const root = createRootRoute({ component: Outlet });
  function Body() {
    const rendered =
      body === "message" ? (
        <MessageBody content={content} previews={previews} />
      ) : (
        <MfmContent content={content} highlight={highlight} />
      );
    return (
      <LocaleProvider>
        <CollectionsProvider value={bundle}>
          {directory ? (
            <MfmChannelsProvider channels={visible} ready={ready}>
              {rendered}
            </MfmChannelsProvider>
          ) : (
            rendered
          )}
        </CollectionsProvider>
      </LocaleProvider>
    );
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
  return { router, messagesFor };
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
    expect(screen.getByRole("link", { name: "アメシュさん: #secret" })).toBeInTheDocument();
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

  it("同期が終わって一覧に現れたチャンネルは、そのまま表示に切り替わる", async () => {
    const { messagesFor } = setup({
      content: link("c-general", "?message=m-general"),
      visible: [],
      ready: false,
    });
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(messagesFor).not.toHaveBeenCalled();
    expect(screen.queryByText("メッセージを表示できません")).toBeNull();
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
  it("遅延表示（defer）では、画面の近くに来た行だけがメッセージを取得する", async () => {
    const bundle = createStoryCollections({
      currentUserId: "u-alice",
      users,
      channels: [general],
      messages,
    });
    const messagesFor = vi.fn(bundle.messages);
    bundle.messages = messagesFor as typeof bundle.messages;
    const root = createRootRoute({ component: Outlet });
    const home = createRoute({
      getParentRoute: () => root,
      path: "/",
      component: () => (
        <LocaleProvider>
          <CollectionsProvider value={bundle}>
            <MfmChannelsProvider channels={[general]}>
              <div data-testid="pane" style={{ height: 200, overflowY: "auto" }}>
                <div style={{ height: 3000 }} />
                <LinkPreviews defer content={link("c-general", "?message=m-general")} />
              </div>
            </MfmChannelsProvider>
          </CollectionsProvider>
        </LocaleProvider>
      ),
    });
    const router = createRouter({
      routeTree: root.addChildren([home]),
      history: createMemoryHistory({ initialEntries: ["/"] }),
    });
    render(<RouterProvider router={router} />);
    const pane = await screen.findByTestId("pane");
    await new Promise((resolve) => setTimeout(resolve, 150));
    // 画面から遠い間は、取得せず、読み込み中の枠だけを置く。
    expect(messagesFor).not.toHaveBeenCalled();
    expect(screen.queryByText(/アメシュ/)).toBeNull();

    pane.scrollTop = pane.scrollHeight;
    expect(await screen.findByRole("link", { name: /アメシュ.*テスト/ })).toBeInTheDocument();
    expect(messagesFor).toHaveBeenCalledWith("c-general");
  });
});
