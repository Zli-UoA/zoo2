import type { Meta, StoryObj } from "@storybook/react-vite";
import { Link } from "@tanstack/react-router";
import { delay, http, HttpResponse } from "msw";
import { expect, fireEvent, fn, screen, userEvent, waitFor, within } from "storybook/test";

import { userAvatarHandler } from "../../../../.storybook/avatar-fixtures";
import {
  storyChannels,
  storyCollections,
  storyDmChannels,
  storyDmMembers,
} from "../../../../.storybook/fixtures";
import { playForImplementation } from "../../../../.storybook/play-for-implementation";
import { StoryShell } from "../../../../.storybook/shell";
import { storySurfaces } from "../../../../.storybook/story-surfaces";
import type { SearchFileItem, SearchMessageItem, SearchPage } from "./globalSearch";
import { SearchScreen } from "./SearchScreen";

const general = storyChannels[0]!;
const author = (id: string, name: string, extra: { is_bot?: boolean; app_id?: string } = {}) => ({
  id,
  user_name: id.replace("user-", ""),
  display_name: name,
  avatar_url: `https://avater.zliusercontent.com/story-${id}.png`,
  is_bot: extra.is_bot ?? false,
  app_id: extra.app_id ?? null,
});

const messageItems: SearchMessageItem[] = [
  {
    id: "search-message-1",
    channel_id: general.id,
    channel_name: general.channel_name,
    is_dm: false,
    parent_id: null,
    user_id: "user-alice",
    author: author("user-alice", "Alice"),
    emojis: {},
    snippet: "明日の**会議**の資料を共有しました。確認をお願いします 🙏",
    created_at: "2026-11-20T09:15:00.000",
  },
  {
    id: "search-message-2",
    channel_id: "channel-design",
    channel_name: "design",
    is_dm: false,
    parent_id: "thread-root-1",
    user_id: "user-bob",
    author: author("user-bob", "Bob"),
    emojis: {},
    snippet: "…会議で出た配色の案を、このスレッドにまとめています。",
    created_at: "2026-11-18T14:02:00.000",
  },
  {
    id: "search-message-3",
    channel_id: storyDmChannels[0]!.id,
    channel_name: storyDmChannels[0]!.channel_name,
    is_dm: true,
    parent_id: null,
    user_id: "user-bob",
    author: author("user-bob", "Bob"),
    emojis: {},
    snippet: "会議の前に 5 分だけ話せますか？",
    created_at: "2026-11-02T08:00:00.000",
  },
  {
    id: "search-message-4",
    channel_id: general.id,
    channel_name: general.channel_name,
    is_dm: false,
    parent_id: null,
    user_id: "user-reminder-app",
    author: author("user-reminder-app", "Reminder", { is_bot: true, app_id: "app-reminder" }),
    emojis: {},
    snippet: "10:00 から定例会議です。",
    created_at: "2026-10-30T00:55:00.000",
  },
];
const olderMessage: SearchMessageItem = {
  ...messageItems[0]!,
  id: "search-message-older",
  snippet: "昨年の会議メモ",
  created_at: "2025-06-01T03:00:00.000",
};

const fileItems: SearchFileItem[] = [
  {
    id: "search-file-1",
    message_id: "message-with-file-1",
    channel_id: general.id,
    channel_name: general.channel_name,
    is_dm: false,
    parent_id: null,
    user_id: "user-alice",
    author: author("user-alice", "Alice"),
    file_name: "会議資料_2026年11月.pdf",
    content_type: "application/pdf",
    file_size: 2_457_600,
    width: null,
    height: null,
    is_sensitive: false,
    snippet: "会議資料_2026年11月.pdf",
    created_at: "2026-11-19T10:00:00.000",
  },
  {
    id: "search-file-2",
    message_id: "message-with-file-2",
    channel_id: "channel-design",
    channel_name: "design",
    is_dm: false,
    parent_id: "thread-root-1",
    user_id: "user-bob",
    author: author("user-bob", "Bob"),
    file_name: "会議ホワイトボード.png",
    content_type: "image/png",
    file_size: 384_000,
    width: 1600,
    height: 900,
    is_sensitive: false,
    snippet: "会議ホワイトボード.png",
    created_at: "2026-11-12T06:30:00.000",
  },
];

const longJapanese: SearchMessageItem[] = [
  {
    ...messageItems[0]!,
    id: "search-long-1",
    author: author(
      "user-alice",
      "とても長い表示名を設定している利用者のアリス（企画開発部・第二グループ）",
    ),
    channel_name: "全社横断プロジェクト推進ワーキンググループの連絡用チャンネル",
    snippet:
      "…来週の全体会議に向けて、各部署から提出された資料を取りまとめ、論点の整理と優先順位付けを行いました。予算の見直し、人員配置の変更、新しい評価制度の導入時期について、それぞれの部署の意見が分かれているため、会議の前に個別に確認しておきたい点があります。特に、第三四半期の実績を踏まえた計画の修正案は…",
  },
  {
    ...messageItems[1]!,
    id: "search-long-2",
    snippet:
      "https://example.com/very/long/path/that/keeps/going/without/any/spaces/会議/資料/2026/11/21/final-final-v3.pdf",
  },
];

/** リクエストを記録して、Story ごとの応答を返す。 */
let requests: URL[] = [];
function searchHandler(respond: (url: URL) => Promise<Response> | Response) {
  return http.get("*/api/search", ({ request }) => {
    const url = new URL(request.url);
    requests.push(url);
    return respond(url);
  });
}
const json = (page: SearchPage) => HttpResponse.json(page);
const resetRequests = () => {
  requests = [];
};

const regionName = "検索";
/** 検索語の `<mark>` で文字が分かれていても、要素の文字全体で探す（いちばん内側の要素だけ）。 */
const fullText = (text: string) => (_: string, element: Element | null) =>
  element?.textContent === text &&
  ![...element.children].some((child) => child.textContent === text);
async function results() {
  const region = await screen.findByRole("region", { name: regionName });
  await waitFor(() => expect(within(region).queryByRole("status")).not.toBeInTheDocument());
  return within(region);
}

const meta = {
  title: "Pages/Search",
  component: SearchScreen,
  parameters: {
    a11y: { test: "error" },
    collections: {
      ...storyCollections,
      channels: [...storyChannels, ...storyDmChannels],
      channelUsers: [...storyCollections.channelUsers, ...storyDmMembers],
    },
    router: { initialEntries: ["/search?q=会議"] },
    msw: {
      handlers: [
        userAvatarHandler,
        searchHandler((url) =>
          json(
            url.searchParams.get("cursor")
              ? { items: [olderMessage], next_cursor: null, searched_until: "2024-12" }
              : {
                  // 既定（すべて）はメッセージとファイルが新しい順に混ざる。
                  items: [messageItems[0]!, fileItems[0]!, ...messageItems.slice(1)],
                  next_cursor: "cursor-1",
                  searched_until: "2025-12",
                },
          ),
        ),
      ],
    },
  },
  beforeEach: resetRequests,
  decorators: [
    (Story) => (
      <StoryShell selectedChannelId={null} initialPane="chat">
        <div className={storySurfaces["Pages/Search"]}>
          <Story />
        </div>
      </StoryShell>
    ),
  ],
} satisfies Meta<typeof SearchScreen>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Prompt: Story = {
  parameters: { router: { initialEntries: ["/search"] } },
  play: playForImplementation(async () => {
    const region = await screen.findByRole("region", { name: regionName });
    await expect(within(region).getByText(/検索する語を入力してください/)).toBeVisible();
    await waitFor(() => expect(within(region).getByRole("searchbox")).toHaveFocus());
    await expect(requests).toHaveLength(0);
  }),
};

export const SubmitQuery: Story = {
  parameters: { router: { initialEntries: ["/search"] } },
  play: playForImplementation(async () => {
    const region = within(await screen.findByRole("region", { name: regionName }));
    await userEvent.type(region.getByRole("searchbox"), "  会議  {Enter}");
    await expect(await region.findByText(/明日の/)).toBeVisible();
    await expect(requests.map((url) => url.searchParams.get("q"))).toEqual(["会議"]);
    // 同じ語の再送信は、取り直しの 1 回だけ。
    await userEvent.click(region.getByRole("button", { name: "検索" }));
    await waitFor(() => expect(requests).toHaveLength(2));
    await expect(requests[1]!.searchParams.get("cursor")).toBeNull();
  }),
};

export const All: Story = {
  play: playForImplementation(async () => {
    const region = await results();
    const list = region.getByRole("list", { name: "すべて" });
    await expect(within(list).getAllByRole("listitem")).toHaveLength(messageItems.length + 1);
    await expect(region.getByText(fullText("会議資料_2026年11月.pdf"))).toBeVisible();
    await expect(requests[0]!.searchParams.get("type")).toBe("all");
    await expect(region.getByText("スレッドの返信")).toBeVisible();
    await expect(region.getByText("2025-12 まで検索しました")).toBeVisible();
    // 返信はスレッドを開き、その返信へ移動する。
    const reply = within(list.children[2] as HTMLElement).getByRole("link");
    await expect(reply.getAttribute("href")).toContain("thread=thread-root-1");
    await expect(reply.getAttribute("href")).toContain("message=search-message-2");
    await expect(requests).toHaveLength(1);
  }),
};

/** 本文の抜粋とファイル名の中で、検索語に当たった部分だけを強調する。 */
export const Highlight: Story = {
  play: playForImplementation(async () => {
    const region = await results();
    const list = region.getByRole("list", { name: "すべて" });
    const marks = list.querySelectorAll("mark[data-search-highlight]");
    await expect(marks.length).toBeGreaterThan(0);
    for (const mark of marks) await expect(mark.textContent).toBe("会議");
    // MFM の太字の中でも強調し、太字は保つ。
    const first = within(list.children[0] as HTMLElement);
    await expect(first.getByText("会議").closest("strong")).not.toBeNull();
    // ファイル名の中でも強調する。
    const file = list.children[1] as HTMLElement;
    await expect(file.querySelector("mark")?.textContent).toBe("会議");
  }),
};

/** 本文を押しても開く。`AppShell` と同じく、画面全体が `<main tabIndex={-1}>` の中にあっても。 */
export const OpenFromRow: Story = {
  decorators: [
    (Story) => (
      <main tabIndex={-1}>
        <Story />
      </main>
    ),
  ],
  play: playForImplementation(async () => {
    const region = await results();
    const row = region.getByRole("list", { name: "すべて" }).children[0] as HTMLElement;
    const destination = within(row).getByRole("link");
    const navigate = fn((event: Event) => event.preventDefault());
    destination.addEventListener("click", navigate);
    try {
      await userEvent.click(within(row).getByText(/明日の/));
      await expect(navigate).toHaveBeenCalledTimes(1);
    } finally {
      destination.removeEventListener("click", navigate);
    }
  }),
};

/** 続きは利用者の操作でだけ読み、前の応答のカーソルを送る。 */
export const LoadOlder: Story = {
  play: playForImplementation(async () => {
    const region = await results();
    await expect(requests).toHaveLength(1);
    await userEvent.click(region.getByRole("button", { name: "さらに古い結果を検索" }));
    await expect(await region.findByText(fullText("昨年の会議メモ"))).toBeVisible();
    await expect(requests).toHaveLength(2);
    await expect(requests[1]!.searchParams.get("cursor")).toBe("cursor-1");
    await expect(region.queryByRole("button", { name: "さらに古い結果を検索" })).toBeNull();
  }),
};

export const Files: Story = {
  parameters: {
    router: { initialEntries: ["/search?q=会議&type=files"] },
    msw: {
      handlers: [
        userAvatarHandler,
        searchHandler((url) =>
          url.searchParams.get("type") === "files"
            ? json({ items: fileItems, next_cursor: null, searched_until: "2025-12" })
            : json({ items: messageItems, next_cursor: null, searched_until: "2025-12" }),
        ),
      ],
    },
  },
  play: playForImplementation(async () => {
    const region = await results();
    await expect(region.getByText(fullText("会議資料_2026年11月.pdf"))).toBeVisible();
    const fileLink = within(region.getByRole("list", { name: "ファイル" })).getAllByRole(
      "link",
    )[0]!;
    await expect(fileLink.getAttribute("href")).toContain("message=message-with-file-1");
    await expect(requests.map((url) => url.searchParams.get("type"))).toEqual(["files"]);
  }),
};

/** 送信者・チャンネル・期間で絞り込むと、その条件で 1 回ずつ取り直す。リセットで元に戻す。 */
export const Filters: Story = {
  play: playForImplementation(async () => {
    const region = await results();
    await userEvent.selectOptions(region.getByRole("combobox", { name: "送信者" }), "user-alice");
    await waitFor(() => expect(requests.at(-1)!.searchParams.get("user_id")).toBe("user-alice"));
    await userEvent.selectOptions(region.getByRole("combobox", { name: "チャンネル" }), general.id);
    await waitFor(() => expect(requests.at(-1)!.searchParams.get("channel_id")).toBe(general.id));
    await userEvent.selectOptions(region.getByRole("combobox", { name: "期間" }), "last7");
    await waitFor(() => expect(requests.at(-1)!.searchParams.get("after")).toMatch(/^\d{4}-/));
    await userEvent.selectOptions(region.getByRole("combobox", { name: "期間" }), "custom");
    await expect(region.getByLabelText("開始日")).toBeVisible();
    // リセットすると条件のない検索に戻る（最初の結果は手元にあるので取り直さない）。
    const before = requests.length;
    await userEvent.click(region.getByRole("button", { name: "条件をリセット" }));
    await waitFor(() => expect(region.getByRole("combobox", { name: "送信者" })).toHaveValue(""));
    await expect(region.getByRole("combobox", { name: "期間" })).toHaveValue("anytime");
    await expect(region.queryByRole("button", { name: "条件をリセット" })).toBeNull();
    await expect(requests).toHaveLength(before);
  }),
};

/** サーバーが拒否する語数・文字数は送らず、理由を示す。再試行は出さない。 */
export const TooManyTerms: Story = {
  parameters: { router: { initialEntries: ["/search"] } },
  play: playForImplementation(async () => {
    const region = within(await screen.findByRole("region", { name: regionName }));
    await userEvent.type(region.getByRole("searchbox"), "how to set up dev server{Enter}");
    await expect(await region.findByRole("alert")).toHaveTextContent(
      "検索できる語は 5 個までです。",
    );
    await expect(region.queryByRole("button", { name: "再試行" })).toBeNull();
    await expect(requests).toHaveLength(0);
    // 入力を直すと理由は消える。
    await userEvent.type(region.getByRole("searchbox"), "{Backspace}");
    await expect(region.queryByRole("alert")).toBeNull();
  }),
};

/** URL で届いた長すぎる語も送らない。 */
export const TermTooLong: Story = {
  parameters: { router: { initialEntries: [`/search?q=${"x".repeat(101)}`] } },
  play: playForImplementation(async () => {
    const region = within(await screen.findByRole("region", { name: regionName }));
    await expect(await region.findByRole("alert")).toHaveTextContent(
      "1 つの語は 100 文字までです。",
    );
    await expect(region.queryByRole("button", { name: "再試行" })).toBeNull();
    await expect(requests).toHaveLength(0);
  }),
};

/** 日付は入力中には検索せず、Enter か入力欄を離れたときに 1 回だけ検索する。空にしても入力欄は残る。 */
export const CustomDates: Story = {
  parameters: { router: { initialEntries: ["/search?q=会議&after=2026-11-01"] } },
  play: playForImplementation(async () => {
    const region = await results();
    const after = region.getByLabelText("開始日");
    const end = region.getByLabelText("終了日");
    const before = requests.length;
    // 年の入力途中の値（ブラウザーは有効な日付として通知する）では検索しない。
    await fireEvent.change(after, { target: { value: "0002-11-01" } });
    await fireEvent.change(after, { target: { value: "2025-11-01" } });
    await expect(requests).toHaveLength(before);
    await fireEvent.keyDown(after, { key: "Enter" });
    await waitFor(() => expect(requests.at(-1)!.searchParams.get("after")).toBe("2025-11-01"));
    await expect(requests).toHaveLength(before + 1);
    await fireEvent.change(end, { target: { value: "2026-11-21" } });
    await expect(requests).toHaveLength(before + 1);
    await fireEvent.focusOut(end);
    // 終了日はその日を含む（API の before は翌日）。
    await waitFor(() => expect(requests.at(-1)!.searchParams.get("before")).toBe("2026-11-22"));
    // 日付を空にしても、日付の指定は表示したまま。
    await fireEvent.change(after, { target: { value: "" } });
    await fireEvent.focusOut(after);
    await fireEvent.change(end, { target: { value: "" } });
    await fireEvent.focusOut(end);
    await waitFor(() => expect(requests.at(-1)!.searchParams.get("before")).toBeNull());
    await expect(requests.at(-1)!.searchParams.get("after")).toBeNull();
    await expect(region.getByRole("combobox", { name: "期間" })).toHaveValue("custom");
    await expect(region.getByLabelText("開始日")).toBeVisible();
  }),
};

/** 開始日が終了日より後なら確定せず、理由を示す。同じ日はよい。 */
export const ReversedDates: Story = {
  parameters: { router: { initialEntries: ["/search?q=会議&after=2026-11-01&before=2026-11-11"] } },
  play: playForImplementation(async () => {
    const region = await results();
    const after = region.getByLabelText("開始日");
    const before = requests.length;
    await fireEvent.change(after, { target: { value: "2026-11-20" } });
    await fireEvent.focusOut(after);
    await expect(await region.findByRole("alert")).toHaveTextContent("開始日は終了日と同じか");
    await expect(after).toHaveValue("2026-11-20");
    await expect(requests).toHaveLength(before);
    await fireEvent.change(after, { target: { value: "2026-11-10" } });
    await fireEvent.focusOut(after);
    await waitFor(() => expect(requests.at(-1)!.searchParams.get("after")).toBe("2026-11-10"));
    await expect(region.queryByRole("alert")).toBeNull();
  }),
};

/** 拒否した理由は、別の検索へ移ったら消す。 */
export const RejectedThenNavigate: Story = {
  decorators: [
    (Story) => (
      <>
        <Link to="/search" search={{ q: "資料" }}>
          別の検索
        </Link>
        <Story />
      </>
    ),
  ],
  play: playForImplementation(async () => {
    const region = await results();
    const box = region.getByRole("searchbox");
    await userEvent.clear(box);
    await userEvent.type(box, "how to set up dev server{Enter}");
    await expect(await region.findByRole("alert")).toBeVisible();
    await userEvent.click(screen.getByRole("link", { name: "別の検索" }));
    await waitFor(() => expect(region.getByRole("searchbox")).toHaveValue("資料"));
    await waitFor(() => expect(region.queryByRole("alert")).toBeNull());
    await expect(region.getByRole("searchbox")).not.toHaveAttribute("aria-invalid");
  }),
};

/** 狭い画面でも、一致の前が長い抜粋で、最初の一致が表示範囲に入る。 */
export const NarrowLateHit: Story = {
  decorators: [
    (Story) => (
      <div style={{ width: 320 }}>
        <Story />
      </div>
    ),
  ],
  parameters: {
    msw: {
      handlers: [
        userAvatarHandler,
        searchHandler(() =>
          json({
            items: [
              {
                ...messageItems[0]!,
                snippet: `…${"今週の定例で話し合った内容を整理しました".repeat(2)}会議の資料`,
              },
            ],
            next_cursor: null,
            searched_until: "2025-12",
          }),
        ),
      ],
    },
  },
  play: playForImplementation(async () => {
    const region = await results();
    const row = region.getByRole("list", { name: "すべて" }).children[0] as HTMLElement;
    const body = row.querySelector(".mfm-body")!;
    const mark = row.querySelector("mark[data-search-highlight]")!;
    await expect(mark.textContent).toBe("会議");
    await expect(mark.getBoundingClientRect().bottom).toBeLessThanOrEqual(
      body.getBoundingClientRect().bottom + 1,
    );
  }),
};

/** 種類を変えると、その種類で 1 回だけ取り直す。 */
export const SwitchKind: Story = {
  parameters: Files.parameters,
  play: playForImplementation(async () => {
    const region = await results();
    await expect(region.getByText(fullText("会議資料_2026年11月.pdf"))).toBeVisible();
    await userEvent.click(region.getByRole("button", { name: "メッセージ" }));
    await expect(await region.findByText(/明日の/)).toBeVisible();
    await expect(requests.map((url) => url.searchParams.get("type"))).toEqual([
      "files",
      "messages",
    ]);
  }),
};

export const Loading: Story = {
  parameters: {
    msw: {
      handlers: [
        userAvatarHandler,
        searchHandler(async () => {
          await delay("infinite");
          return json({ items: [], next_cursor: null, searched_until: null });
        }),
      ],
    },
  },
  play: playForImplementation(async () => {
    const region = within(await screen.findByRole("region", { name: regionName }));
    await expect(await region.findByRole("status")).toHaveTextContent("検索中…");
  }),
};

export const Empty: Story = {
  parameters: {
    msw: {
      handlers: [
        userAvatarHandler,
        searchHandler(() => json({ items: [], next_cursor: null, searched_until: "2024-03" })),
      ],
    },
  },
  play: playForImplementation(async () => {
    const region = await results();
    await expect(region.getByText("見つかりませんでした。")).toBeVisible();
    await expect(region.queryByRole("button", { name: "さらに古い結果を検索" })).toBeNull();
  }),
};

export const EmptyWithOlder: Story = {
  parameters: {
    msw: {
      handlers: [
        userAvatarHandler,
        searchHandler((url) =>
          json(
            url.searchParams.get("cursor")
              ? { items: [olderMessage], next_cursor: null, searched_until: "2024-12" }
              : { items: [], next_cursor: "cursor-1", searched_until: "2025-12" },
          ),
        ),
      ],
    },
  },
  play: playForImplementation(async () => {
    const region = await results();
    await expect(region.getByText(/2025-12 までに見つかりませんでした/)).toBeVisible();
    await expect(region.getByRole("button", { name: "さらに古い結果を検索" })).toBeEnabled();
    await expect(requests).toHaveLength(1);
  }),
};

export const LoadError: Story = {
  parameters: {
    msw: {
      handlers: [
        userAvatarHandler,
        searchHandler(() =>
          requests.length === 1
            ? new HttpResponse(null, { status: 500 })
            : json({ items: messageItems, next_cursor: null, searched_until: "2025-12" }),
        ),
      ],
    },
  },
  play: playForImplementation(async () => {
    const region = within(await screen.findByRole("region", { name: regionName }));
    await expect(await region.findByRole("alert")).toHaveTextContent("検索できませんでした。");
    await userEvent.click(region.getByRole("button", { name: "再試行" }));
    await expect(await region.findByText(/明日の/)).toBeVisible();
    await expect(requests).toHaveLength(2);
  }),
};

export const LongJapanese: Story = {
  parameters: {
    msw: {
      handlers: [
        userAvatarHandler,
        searchHandler(() =>
          json({ items: longJapanese, next_cursor: "cursor-1", searched_until: "2025-12" }),
        ),
      ],
    },
  },
  play: playForImplementation(async () => {
    const region = await results();
    const list = region.getByRole("list", { name: "すべて" });
    for (const row of Array.from(list.children))
      await expect(row.scrollWidth).toBeLessThanOrEqual(row.clientWidth + 1);
  }),
};

export const Mobile: Story = {
  globals: { viewport: { value: "mobile" } },
  play: All.play,
};
