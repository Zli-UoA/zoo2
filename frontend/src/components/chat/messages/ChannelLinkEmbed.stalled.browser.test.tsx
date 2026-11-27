import "../../../styles.css";
import { render, screen } from "@testing-library/react";
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  Outlet,
  RouterProvider,
} from "@tanstack/react-router";
import { act, useState, type ReactNode } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vite-plus/test";

import { E2eeContext, type E2eeContextValue } from "../../../lib/e2ee/e2ee-context";
import { LOCALE_STORAGE_KEY } from "../../../lib/i18n/locale";
import { LocaleProvider } from "../../i18n/LocaleProvider";
import { ChannelLinkEmbed, LINK_LOAD_TIMEOUT_MS } from "./ChannelLinkEmbed";
import type { ChannelDirectory } from "./MfmChannels";

/**
 * `useLiveQuery` は、取得が失敗しても（サーバーのエラー・オフライン）`isError` にならず、`isLoading` のまま
 * 再試行を続ける。その状態を、メッセージの取得をモックして再現する。
 */
const query = vi.hoisted(() => ({
  current: { data: [] as unknown[], isLoading: true },
}));
vi.mock("../../../hooks/useThread", () => ({
  useLinkedMessageRows: () => query.current,
  useChannelHasCiphertext: () => false,
}));

const directory: ChannelDirectory = {
  ready: true,
  byId: new Map([
    [
      "c-general",
      {
        id: "c-general",
        name: "general",
        description: null,
        isPrivate: false,
        isDm: false,
        encrypted: false,
      },
    ],
  ]),
};
const target = { channelId: "c-general", search: { message: "m-1" } };

const row = {
  id: "m-1",
  user_id: "u-bob",
  content: "遅れて届いた本文",
  parent_id: null,
  created_at: "2026-11-23 07:30:00.000",
  display_name: "アメシュ",
  user_name: "amesyu",
  avatar_url: null,
  is_bot: false,
  deleted_at: null,
  card: null,
  components: null,
  app_blocks: null,
  ciphertext: null,
};

/** `node` を描く。返す `refresh` で、同じ場所を（モックの結果を変えたあとに）描き直す。 */
function mount(node: () => ReactNode) {
  let bump = () => {};
  function Host() {
    const [, setVersion] = useState(0);
    bump = () => setVersion((version) => version + 1);
    return <LocaleProvider>{node()}</LocaleProvider>;
  }
  const root = createRootRoute({ component: Outlet });
  const home = createRoute({ getParentRoute: () => root, path: "/", component: Host });
  const router = createRouter({
    routeTree: root.addChildren([home]),
    history: createMemoryHistory({ initialEntries: ["/"] }),
  });
  const result = render(<RouterProvider router={router} />);
  return {
    ...result,
    refresh: () =>
      act(async () => {
        bump();
      }),
  };
}

beforeEach(() => {
  localStorage.setItem(LOCALE_STORAGE_KEY, "ja");
  query.current = { data: [], isLoading: true };
  vi.useFakeTimers({ shouldAdvanceTime: true });
});

afterEach(() => {
  vi.useRealTimers();
  localStorage.removeItem(LOCALE_STORAGE_KEY);
});

async function advance(ms: number) {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
}

it("取得を始めた直後は、読み上げに「読み込み中」を伝える枠を出す", async () => {
  mount(() => <ChannelLinkEmbed target={target} directory={directory} />);
  const status = await screen.findByRole("status");
  expect(status).toHaveTextContent("リンク先を読み込み中");
  expect(status.getAttribute("aria-busy")).toBe("true");
});

it("取得が終わらないまま一定時間たつと、枠をやめて、読み込めなかったと書く", async () => {
  mount(() => <ChannelLinkEmbed target={target} directory={directory} />);
  await screen.findByRole("status");

  await advance(LINK_LOAD_TIMEOUT_MS - 1_000);
  expect(screen.getByRole("status")).toBeInTheDocument();
  expect(screen.queryByText("リンク先を読み込めませんでした")).toBeNull();

  await advance(2_000);
  expect(screen.getByText("リンク先を読み込めませんでした")).toBeInTheDocument();
  // 点滅する枠は残さない。見られないという表示とも区別する。
  expect(screen.queryByRole("status")).toBeNull();
  expect(screen.queryByText("メッセージを表示できません")).toBeNull();
});

it("読み込めなかったあとでも、取得できたら、そのまま内容に切り替わる", async () => {
  const { refresh } = mount(() => <ChannelLinkEmbed target={target} directory={directory} />);
  await screen.findByRole("status");
  await advance(LINK_LOAD_TIMEOUT_MS + 1_000);
  expect(screen.getByText("リンク先を読み込めませんでした")).toBeInTheDocument();

  // オンラインに戻るなどで、取得が終わった。
  query.current = { data: [row], isLoading: false };
  await refresh();
  expect(await screen.findByRole("link", { name: /遅れて届いた本文/ })).toBeInTheDocument();
  expect(screen.queryByText("リンク先を読み込めませんでした")).toBeNull();
});

it("取得を始めていない（画面から遠い）間は、時間がたっても、読み込めなかったと書かない", async () => {
  mount(() => <ChannelLinkEmbed target={target} directory={directory} active={false} />);
  await screen.findByRole("status");
  await advance(LINK_LOAD_TIMEOUT_MS * 3);
  expect(screen.getByRole("status")).toBeInTheDocument();
  expect(screen.queryByText("リンク先を読み込めませんでした")).toBeNull();
});

/** 画面の近くにあるか（`active`）を、あとから切り替えられるカードを描く。 */
async function mountToggleable() {
  const control = { active: true };
  const view = mount(() => (
    <ChannelLinkEmbed target={target} directory={directory} active={control.active} />
  ));
  await screen.findByRole("status");
  return async (active: boolean) => {
    control.active = active;
    await view.refresh();
  };
}

it("読み込めなかったあとで、画面から離れて戻っても、枠に戻らない", async () => {
  const setActive = await mountToggleable();
  await advance(LINK_LOAD_TIMEOUT_MS + 1_000);
  expect(screen.getByText("リンク先を読み込めませんでした")).toBeInTheDocument();

  for (const active of [false, true]) {
    await setActive(active);
    expect(screen.getByText("リンク先を読み込めませんでした")).toBeInTheDocument();
    expect(screen.queryByRole("status")).toBeNull();
  }
});

it("取得を始めたあとは、画面から離れても、経過した時間を数え続ける", async () => {
  // 画面から離れても、タブを隠しても、始めた取得は続く。
  const setActive = await mountToggleable();
  await advance(LINK_LOAD_TIMEOUT_MS / 2);
  await setActive(false);
  expect(screen.getByRole("status")).toBeInTheDocument();
  await advance(LINK_LOAD_TIMEOUT_MS / 2 + 1_000);
  expect(screen.getByText("リンク先を読み込めませんでした")).toBeInTheDocument();
});

it("チャンネルの一覧の同期を待っている間は、時間がたっても、読み込めなかったと書かない", async () => {
  // 一覧にないチャンネルは、同期が終わるまで、メッセージの取得を始められない。取得の失敗ではない。
  const waiting: ChannelDirectory = { ready: false, byId: new Map() };
  mount(() => <ChannelLinkEmbed target={target} directory={waiting} />);
  await screen.findByRole("status");
  await advance(LINK_LOAD_TIMEOUT_MS * 3);
  expect(screen.getByRole("status")).toBeInTheDocument();
  expect(screen.queryByText("リンク先を読み込めませんでした")).toBeNull();
  expect(screen.queryByText("メッセージを表示できません")).toBeNull();
});

it("取得が終われば、時間がたっても、読み込めなかったとは書かない", async () => {
  query.current = { data: [row], isLoading: false };
  mount(() => <ChannelLinkEmbed target={target} directory={directory} />);
  expect(await screen.findByRole("link", { name: /遅れて届いた本文/ })).toBeInTheDocument();
  await advance(LINK_LOAD_TIMEOUT_MS * 3);
  expect(screen.queryByText("リンク先を読み込めませんでした")).toBeNull();
});

it("暗号化かどうかの確認が終わらないまま一定時間たつと、本文を出さず、読み込めなかったと書く", async () => {
  // 取得は終わっているが、暗号ストアが答えない。本文は出さない。
  query.current = { data: [row], isLoading: false };
  const e2ee = {
    worker: { request: () => new Promise(() => {}) },
  } as unknown as E2eeContextValue;
  mount(() => (
    <E2eeContext.Provider value={e2ee}>
      <ChannelLinkEmbed target={target} directory={directory} />
    </E2eeContext.Provider>
  ));
  await screen.findByRole("status");
  expect(document.body.textContent).not.toContain("遅れて届いた本文");
  await advance(LINK_LOAD_TIMEOUT_MS + 1_000);
  expect(screen.getByText("リンク先を読み込めませんでした")).toBeInTheDocument();
  expect(document.body.textContent).not.toContain("遅れて届いた本文");
});

it("読み込み中の枠が要らない画面（showFallback が false）では、何も出さない", async () => {
  const { container } = mount(() => (
    <ChannelLinkEmbed target={target} directory={directory} showFallback={false} />
  ));
  await advance(LINK_LOAD_TIMEOUT_MS * 2);
  expect(screen.queryByRole("status")).toBeNull();
  expect(container.textContent).not.toContain("読み込めませんでした");
});
