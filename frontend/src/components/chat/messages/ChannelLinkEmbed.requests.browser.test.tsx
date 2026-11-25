import "../../../styles.css";
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  Outlet,
  RouterProvider,
} from "@tanstack/react-router";
import { useState } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vite-plus/test";

import { resolveConfig } from "../../../lib/config";
import { getCollections } from "../../../lib/electric/collections";
import { CollectionsProvider } from "../../../lib/electric/collections-context";
import type { ChannelRow } from "../../../lib/electric/types";
import { LOCALE_STORAGE_KEY } from "../../../lib/i18n/locale";
import { LocaleProvider } from "../../i18n/LocaleProvider";
import { LINK_LOAD_TIMEOUT_MS } from "./ChannelLinkEmbed";
import { MessageBody } from "./MessageRow";
import { MfmChannelsProvider } from "./MfmChannels";

/**
 * 実際の Electric コレクション（`messages` は on-demand）に、`fetch` の代役をつないで、
 * メッセージリンクの埋め込みが出す要求を数える（docs/request-design.md の確認項目）。
 */

const origin = window.location.origin;
const channelId = "c-general";
const hiddenId = "c-hidden";

function channel(id: string, overrides: Partial<ChannelRow> = {}): ChannelRow {
  return {
    id,
    channel_name: id === channelId ? "general" : "hidden",
    description: null,
    created_user_id: "u1",
    is_private: false,
    is_dm: false,
    created_at: "2026-01-01 00:00:00.000",
    updated_at: null,
    archive_at: null,
    deleted_at: null,
    imported_from_slack: false,
    slack_import_kind: null,
    slack_imported_at: null,
    ...overrides,
  };
}

const display = JSON.stringify({
  author: { id: "u1", user_name: "bob", display_name: "アメシュ", avatar_url: null },
  emojis: {},
  attachments: [],
  reactions: [],
  reply_count: 0,
});
const rows = ["m-1", "m-2", "m-3"].map((id, index) => ({
  id,
  user_id: "u1",
  channel_id: channelId,
  content: `本文 ${id}`,
  created_at: `2026-01-01 00:0${index}:00.000`,
  updated_at: null,
  deleted_at: null,
  parent_id: null as string | null,
  display_data: display,
  message_kind: "message",
  broadcast: "false",
  apub: "false",
}));

const headers = {
  "content-type": "application/json",
  "electric-handle": "h-messages",
  "electric-offset": "0_0",
  "electric-schema": JSON.stringify({ id: { type: "text" } }),
};

/** `"id" = $n` の subset snapshot を再現する。 */
function subsetRows(params: URLSearchParams) {
  const values = JSON.parse(params.get("subset__params") ?? "{}") as Record<string, string>;
  const where = params.get("subset__where") ?? "";
  const byId = /"id"\s*=\s*\$(\d+)/.exec(where);
  return byId ? rows.filter((row) => row.id === values[byId[1]!]) : [];
}

interface Recorded {
  /** `messages` Shape への要求。 */
  messages: URLSearchParams[];
  /** そのうち、メッセージの行を取りにいく要求（subset snapshot）。 */
  snapshots: URLSearchParams[];
}

function stubFetch(options: { snapshotStatus?: number; messagesStatus?: number } = {}): Recorded {
  const recorded: Recorded = { messages: [], snapshots: [] };
  vi.stubGlobal(
    "fetch",
    vi.fn<typeof fetch>((input, init) => {
      const url = new URL(
        typeof input === "string" ? input : input instanceof URL ? input.href : input.url,
        window.location.href,
      );
      const hang = () =>
        new Promise<Response>((_resolve, reject) => {
          init?.signal?.addEventListener("abort", () => reject(init.signal?.reason), {
            once: true,
          });
        });
      const live = url.searchParams.get("live") === "true" || url.searchParams.has("live_sse");
      if (!url.pathname.endsWith("/messages")) {
        // ほかの Shape は、空の同期済みの応答を返す（変更の購読は取得中のまま）。
        if (live) return hang();
        return Promise.resolve(
          new Response(JSON.stringify([{ headers: { control: "up-to-date" } }]), {
            headers: { ...headers, "electric-handle": `h-${url.pathname}` },
          }),
        );
      }
      recorded.messages.push(url.searchParams);
      // `messages` Shape への要求が、すべて失敗する（権限がないなど）。
      if (options.messagesStatus) {
        return Promise.resolve(new Response("error", { status: options.messagesStatus }));
      }
      if (!url.searchParams.has("subset__where") && !url.searchParams.has("subset__limit")) {
        return hang();
      }
      recorded.snapshots.push(url.searchParams);
      // 取得の失敗（サーバーのエラー）を再現する。
      if (options.snapshotStatus) {
        return Promise.resolve(new Response("error", { status: options.snapshotStatus }));
      }
      const data = subsetRows(url.searchParams).map((row) => ({
        key: row.id,
        value: Object.fromEntries(
          Object.entries(row).map(([key, value]) => [key, value === null ? null : String(value)]),
        ),
        headers: { operation: "insert", relation: ["public", "messages"] },
      }));
      return Promise.resolve(
        new Response(
          JSON.stringify({
            metadata: { xmin: "1", xmax: "1", xip_list: [], snapshot_mark: 1, database_lsn: "1" },
            data,
          }),
          { headers },
        ),
      );
    }),
  );
  return recorded;
}

let namespace = 0;
function renderBodies(contents: string[], visible: ChannelRow[]) {
  const bundle = getCollections(
    resolveConfig({ DEV: true, VITE_SHAPE_MODE: "proxy", VITE_DEV_USER_ID: "u1" }),
    { namespace: `channel-link-requests-${(namespace += 1)}`, persistence: null },
  );
  const root = createRootRoute({ component: Outlet });
  const control: { setShown: (shown: boolean) => void } = { setShown: () => {} };
  function Bodies() {
    const [shown, setShown] = useState(true);
    control.setShown = setShown;
    return (
      <LocaleProvider>
        <CollectionsProvider value={bundle}>
          <MfmChannelsProvider channels={visible}>
            {shown
              ? contents.map((content, index) => <MessageBody key={index} content={content} />)
              : null}
          </MfmChannelsProvider>
        </CollectionsProvider>
      </LocaleProvider>
    );
  }
  const home = createRoute({ getParentRoute: () => root, path: "/", component: Bodies });
  const router = createRouter({
    routeTree: root.addChildren([home]),
    history: createMemoryHistory({ initialEntries: ["/"] }),
  });
  render(<RouterProvider router={router} />);
  return {
    bundle,
    /** 投稿を画面から消す・戻す（コレクションはそのまま）。 */
    show: (shown: boolean) => act(async () => control.setShown(shown)),
  };
}

const link = (id: string, message: string) => `${origin}/channels/${id}?message=${message}`;

beforeEach(() => localStorage.setItem(LOCALE_STORAGE_KEY, "ja"));
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  localStorage.removeItem(LOCALE_STORAGE_KEY);
});

/**
 * 本文を描いて、メッセージの行を取る要求（subset snapshot）の数を返す。取り始めの要求の数は
 * Electric クライアントの作りで決まるので、絶対数ではなく、リンクが 1 つのときとの比で確かめる。
 */
async function snapshotCount(contents: string[], visible: ChannelRow[], expectedTexts: string[]) {
  const recorded = stubFetch();
  renderBodies(contents, visible);
  for (const text of expectedTexts) {
    await waitFor(() => expect(screen.getAllByText(text).length).toBeGreaterThan(0), {
      timeout: 5_000,
    });
  }
  // 遅れて出る要求も数える。
  await new Promise((resolve) => setTimeout(resolve, 300));
  cleanup();
  vi.unstubAllGlobals();
  return Object.assign(recorded.snapshots, {
    // 行を取る要求ではない、チャンネルの変更を受け取るための要求（Shape の購読）。
    streams: recorded.messages.filter((params) => !params.has("subset__where")),
  });
}

it("同じメッセージへのリンクが何か所にあっても、メッセージの取得は 1 リンクのときと同じ", async () => {
  const visible = [channel(channelId)];
  const single = await snapshotCount([link(channelId, "m-1")], visible, ["本文 m-1"]);
  expect(single.length).toBeGreaterThan(0);
  // 短くしたリンクと埋め込みが 1 つの投稿に 1 組。それを 3 つの投稿に書いた状態（問い合わせの購読は 6 つ）。
  const many = await snapshotCount(
    [link(channelId, "m-1"), `また ${link(channelId, "m-1")}`, `さらに ${link(channelId, "m-1")}`],
    visible,
    ["本文 m-1"],
  );
  expect(many).toHaveLength(single.length);
  // 同じチャンネルの変更の購読も、リンクの数によらず同じ（チャンネルごとに 1 組）。
  expect(single.streams.length).toBeGreaterThan(0);
  expect(many.streams).toHaveLength(single.streams.length);
});

it("別のメッセージへのリンクは、メッセージごとに取得し、チャンネルの全件は取得しない", async () => {
  const visible = [channel(channelId)];
  const single = await snapshotCount([link(channelId, "m-1")], visible, ["本文 m-1"]);
  const three = await snapshotCount(
    [link(channelId, "m-1"), link(channelId, "m-2"), link(channelId, "m-3")],
    visible,
    ["本文 m-1", "本文 m-2", "本文 m-3"],
  );
  expect(three).toHaveLength(single.length * 3);
  // 同じチャンネルの別のメッセージを増やしても、チャンネルの変更の購読は増えない。
  expect(three.streams).toHaveLength(single.streams.length);
  // どれも、ID を指定した 1 件の取得で、チャンネルの全件や一覧のページを取りにいく要求ではない。
  for (const params of three) {
    expect(params.get("subset__where")).toMatch(/"id"\s*=\s*\$1/);
    expect(params.get("subset__limit")).toBeNull();
  }
});

it("見られないチャンネルのリンクは、メッセージの要求を一切出さない", async () => {
  const recorded = stubFetch();
  renderBodies([link(hiddenId, "m-1"), link(hiddenId, "m-2")], [channel(channelId)]);
  expect(await screen.findAllByText("メッセージを表示できません")).toHaveLength(2);
  await new Promise((resolve) => setTimeout(resolve, 300));
  expect(recorded.messages).toHaveLength(0);
});

it("チャンネルのリンク（メッセージを指さない）は、そのチャンネルの messages を同期しない", async () => {
  // チャンネルの名前と説明は一覧にある。メッセージを取る必要はなく、変更の購読も開かない。
  const recorded = stubFetch();
  renderBodies([`${origin}/channels/${channelId}`], [channel(channelId)]);
  // 本文中の短い表示と、カード（名前と説明）の 2 つが出る。
  expect(await screen.findAllByRole("link", { name: /#general/ })).toHaveLength(2);
  await new Promise((resolve) => setTimeout(resolve, 300));
  expect(recorded.messages).toHaveLength(0);
});

it("リアクションや添付の Shape は、リンク先のチャンネルについて購読しない", async () => {
  stubFetch();
  const fetchMock = vi.mocked(fetch);
  renderBodies([link(channelId, "m-1")], [channel(channelId)]);
  await waitFor(() => expect(screen.getByText("本文 m-1")).toBeInTheDocument(), {
    timeout: 5_000,
  });
  await new Promise((resolve) => setTimeout(resolve, 300));
  const paths = fetchMock.mock.calls.map(
    ([input]) =>
      new URL(
        typeof input === "string" ? input : input instanceof URL ? input.href : input.url,
        window.location.href,
      ).pathname,
  );
  expect(paths.some((path) => path.endsWith("/messages"))).toBe(true);
  expect(paths.some((path) => /message_reactions|attachments/.test(path))).toBe(false);
});

it("サーバーが取得に失敗し続けても、枠を出し続けず、読み込めなかったと書く", async () => {
  // `useLiveQuery` は、取得が失敗しても `isError` にならず、`isLoading` のまま再試行を続ける。
  stubFetch({ snapshotStatus: 500 });
  renderBodies([link(channelId, "m-1")], [channel(channelId)]);
  expect(await screen.findByRole("status")).toHaveTextContent("リンク先を読み込み中");
  expect(
    await screen.findByText("リンク先を読み込めませんでした", undefined, {
      timeout: LINK_LOAD_TIMEOUT_MS + 5_000,
    }),
  ).toBeInTheDocument();
  expect(screen.queryByRole("status")).toBeNull();
  expect(screen.queryByText("メッセージを表示できません")).toBeNull();
}, 30_000);

it.each([401, 403, 400])(
  "同期が再試行されない失敗（HTTP %i）で止まったら、要求を出し続けず、読み込めなかったと書く",
  async (status) => {
    // ライブラリが出す、同期の失敗と、失敗した問い合わせのログ。
    const logged = vi.spyOn(console, "error").mockImplementation(() => {});
    // ほかのテストの再試行の要求が混ざらないように、このテストだけのチャンネルを使う。
    const failing = `c-failing-${status}`;
    const recorded = stubFetch({ messagesStatus: status });
    const requests = () =>
      recorded.messages.filter((params) => params.toString().includes(failing)).length;
    const logs = () =>
      logged.mock.calls.filter((args) => args.map(String).join(" ").includes(failing)).length;
    renderBodies(
      [link(failing, "m-1"), `また ${link(failing, "m-1")}`, link(failing, "m-2")],
      [channel(failing)],
    );
    // 10 秒を待たずに分かる。「表示できません」（見られない）とは区別する。
    await waitFor(() =>
      expect(screen.getAllByText("リンク先を読み込めませんでした")).toHaveLength(3),
    );
    expect(screen.queryByText("メッセージを表示できません")).toBeNull();
    expect(screen.queryByRole("status")).toBeNull();
    await new Promise((resolve) => setTimeout(resolve, 500));
    const settled = { requests: requests(), logs: logs() };
    // 2 つのメッセージの取得と、チャンネルの変更の購読だけ。描画のたびには増えない。
    expect(settled.requests).toBeGreaterThan(0);
    expect(settled.requests).toBeLessThanOrEqual(4);
    expect(settled.logs).toBeGreaterThan(0);
    expect(settled.logs).toBeLessThan(10);
    await new Promise((resolve) => setTimeout(resolve, 1_000));
    // 時間がたっても、要求もログも増えない。
    expect(requests()).toBe(settled.requests);
    expect(logs()).toBe(settled.logs);
    // 本文中のリンクは、URL のまま残る。
    expect(screen.getAllByRole("link")).toHaveLength(3);
    logged.mockRestore();
  },
);

// 取り直しの条件（docs/mfm.md）。失敗が残るのは、リンクの問い合わせではなく、チャンネルの `messages`。
it("同期に失敗したチャンネルは、リンクを出し直しても取り直さず、`messages` が片付いたあとに取り直す", async () => {
  const logged = vi.spyOn(console, "error").mockImplementation(() => {});
  const failing = "c-recover";
  stubFetch({ messagesStatus: 403 });
  const view = renderBodies([link(failing, "m-1")], [channel(failing)]);
  await screen.findByText("リンク先を読み込めませんでした");
  const source = view.bundle.messages(failing);

  // サーバーは回復した。リンクを消して、リンクの問い合わせが片付く（5 秒）まで待ってから、出し直す。
  const recovered = stubFetch();
  await view.show(false);
  await waitFor(() => expect(source.subscriberCount).toBe(0), { timeout: 10_000 });
  await view.show(true);
  expect(await screen.findByText("リンク先を読み込めませんでした")).toBeInTheDocument();
  await new Promise((resolve) => setTimeout(resolve, 500));
  expect(recovered.messages).toHaveLength(0);
  expect(source.status).toBe("error");

  // 使う表示がなくなって 5 分（TanStack DB の既定の `gcTime`）たつと、ライブラリが `messages` を片付ける。
  // 待たずに、同じ片付けを呼ぶ。
  await view.show(false);
  await waitFor(() => expect(source.subscriberCount).toBe(0), { timeout: 10_000 });
  await source.cleanup();
  await view.show(true);
  expect(await screen.findByText("本文 m-1", undefined, { timeout: 5_000 })).toBeInTheDocument();
  logged.mockRestore();
}, 40_000);
