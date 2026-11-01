import type { KotlinPreviewProps } from "./KotlinPreview";
import type { Selection } from "./kotlin-stories";
import type { NativeInteraction as Step } from "./native-interactions";
import { homePosts } from "./home-fixtures";

const button = (name: string, extra: Partial<Step> = {}): Step => ({
  action: "click",
  role: "button",
  name,
  exact: true,
  ...extra,
});
const wait = (text: string, extra: Partial<Step> = {}): Step => ({
  action: "wait",
  text,
  exact: true,
  ...extra,
});
const role = (role: string, name?: string, extra: Partial<Step> = {}): Step => ({
  action: "wait",
  role,
  ...(name === undefined ? {} : { name }),
  exact: true,
  ...extra,
});
const type = (text: string, name = "近況を投稿", extra: Partial<Step> = {}): Step => ({
  action: "type",
  role: "textbox",
  name,
  text,
  ...extra,
});
const menu = (name: string, extra: Partial<Step> = {}): Step => ({
  action: "click",
  role: "menuitemradio",
  name,
  exact: true,
  ...extra,
});
const fixture = (name: string): Step => ({ action: "fixture", name });
const composer = { role: "dialog", name: "近況を投稿" };
const opened: Step[] = [
  button("投稿を書く"),
  role("dialog", "近況を投稿"),
  role("textbox", "近況を投稿"),
];
const closed: Step = role("dialog", "近況を投稿", { state: "hidden" });
const send: Step = button("投稿", { within: composer });
const search = (query: string): Step[] => [
  button("ユーザーを検索"),
  role("dialog", "ユーザーを検索"),
  type(query, "ユーザーを検索"),
  button("検索", { within: { role: "dialog", name: "ユーザーを検索" } }),
];
const operations: Step = button("投稿の操作", {
  within: { text: homePosts[0].content.split("\n")[0], exact: false },
});
const visibility: Step[] = [button("公開範囲: ホーム"), role("menu")];
const heart: Step[] = [
  button("リアクション 0"),
  role("dialog", "リアクション"),
  button("❤️", { within: { role: "group", name: "よく使うリアクション" } }),
];
const reply: Step[] = [button("返信 0"), role("dialog", "近況を投稿"), wait("Aliceさんに返信")];
const defaultChecks: Step[] = [
  wait(homePosts[0].content.split("\n")[0], { exact: false }),
  role("textbox", "近況を投稿", { state: "hidden" }),
  role("button", "投稿を書く"),
  wait("ActivityPub・チャンネル連携", { state: "hidden" }),
  wait("zoo.example/api/activitypub/users", { exact: false, state: "hidden" }),
];

export const homeInteractions: Record<string, Step[]> = {
  default: defaultChecks,
  mobile: defaultChecks,
  empty: [wait("まだ投稿がありません")],
  loading: [role("status"), wait("読み込み中…", { within: { role: "status" } })],
  "load-error": [role("alert"), wait("まだ投稿がありません", { state: "hidden" })],
  submit: [...opened, type("公開する近況"), send, closed, wait("公開する近況")],
  "send-error": [
    ...opened,
    type("残しておく下書き"),
    send,
    role("alert"),
    role("textbox", "近況を投稿", { valueExact: "残しておく下書き" }),
  ],
  "older-posts": [
    { action: "scroll", role: "feed", deltaY: 100000 },
    button("過去の投稿を読む"),
    wait("過去の投稿を読み込みました"),
    role("button", "過去の投稿を読む", { state: "hidden" }),
  ],
  "delete-post": [
    operations,
    { action: "click", role: "menuitem", name: "投稿を削除" },
    role("alertdialog"),
    button("投稿を削除", { within: { role: "alertdialog" } }),
    wait(homePosts[0].content.split("\n")[0], { exact: false, state: "hidden" }),
  ],
  federated: [
    wait("外部のフォロー先から届いた投稿です。"),
    role("img", "ActivityPub", { state: "hidden" }),
    role("link", "#general", { count: 1 }),
    fixture("homeChannelMetadata"),
  ],
  "cross-post": [
    ...opened,
    ...visibility,
    role("menuitemradio", "パブリック", { enabled: true }),
    menu("パブリック"),
    { action: "select", role: "combobox", name: "同期先チャンネル", value: "#general" },
    type("同じ投稿を連合とチャンネルに"),
    send,
    closed,
    wait("同じ投稿を連合とチャンネルに"),
    role("img", "ActivityPub", { state: "hidden" }),
    role("link", "#general"),
    fixture("homeChannelMetadata"),
  ],
  "followers-only": [
    ...opened,
    { action: "select", role: "combobox", name: "同期先チャンネル", value: "#general" },
    ...visibility,
    role("menuitemradio", "フォロワー限定", { enabled: true }),
    menu("フォロワー限定"),
    role("combobox", "同期先チャンネル", { enabled: false, valueExact: "紐づけなし" }),
    role("menu", undefined, { state: "hidden" }),
  ],
  "follow-remote": [
    ...search("@bob@social.example"),
    role("article", "ユーザー詳細"),
    wait("Bob @ social.example", { within: { role: "article", name: "ユーザー詳細" } }),
    button("フォロー", { within: { role: "article", name: "ユーザー詳細" } }),
    wait("承認待ち"),
  ],
  "edit-post": [
    operations,
    { action: "click", role: "menuitem", name: "投稿を編集" },
    { action: "type", role: "textbox", name: "投稿を編集", value: "編集した近況です" },
    button("保存"),
    role("textbox", "投稿を編集", { state: "hidden" }),
    wait("編集した近況です"),
  ],
  "federation-error": [button("ユーザーを検索"), role("dialog", "ユーザーを検索"), role("alert")],
  "visibility-menu": [
    ...opened,
    role("button", undefined, { within: { role: "group", name: "投稿設定" }, count: 3 }),
    fixture("homeToolbar"),
    ...visibility,
    role("menuitemradio", undefined, { count: 3 }),
    role("menuitemradio", "ホーム", { checked: true }),
    fixture("homeVisibilityIcons"),
  ],
  composer: [...opened, role("group", "投稿設定"), fixture("homeComposerDesktop")],
  "mobile-composer": [
    fixture("homeObserveComposerFocus"),
    ...opened,
    fixture("homeComposerMobile"),
  ],
  "draft-preserved": [
    ...opened,
    type("途中まで書いた下書き"),
    button("閉じる", { within: composer }),
    closed,
    ...opened,
    role("textbox", "近況を投稿", { valueExact: "途中まで書いた下書き" }),
  ],
  "federation-toggle": [
    ...opened,
    button("連合あり", { within: composer }),
    role("menuitemradio", "連合あり", { checked: true }),
    menu("連合なし"),
    role("button", "連合なし", { within: composer }),
    role("button", "公開範囲: ホーム"),
    type("連合しないホーム投稿"),
    send,
    closed,
    wait("連合しないホーム投稿"),
  ],
  "federation-unavailable": [
    ...opened,
    button("連合なし", { within: composer }),
    role("menuitemradio", "連合あり", { enabled: false }),
    menu("連合なし"),
    type("連合しないホーム投稿"),
    send,
    closed,
    wait("連合しないホーム投稿"),
  ],
  sending: [
    ...opened,
    type("送信中の投稿"),
    send,
    role("button", "投稿中…", { enabled: false }),
    role("button", "閉じる", { enabled: false }),
    { action: "key", key: "Escape" },
    role("dialog", "近況を投稿"),
  ],
  "user-search": [
    ...search("alice"),
    role("article", "ユーザー詳細"),
    wait("Alice", { within: { role: "article", name: "ユーザー詳細" } }),
    wait("登録日:", { exact: false }),
  ],
  realtime: [
    wait(homePosts[0].content.split("\n")[0], { exact: false }),
    fixture("homeLiveReady"),
    fixture("homeLiveAdd"),
    wait("別端末から届いた新しい投稿"),
    fixture("homeLiveEdit"),
    wait("編集がリアルタイムで反映されます"),
    wait(homePosts[0].content.split("\n")[0], { exact: false, state: "hidden" }),
    fixture("homeLiveReconnect"),
    wait("再接続後の最新投稿"),
  ],
  "pull-to-refresh": [
    wait(homePosts[0].content.split("\n")[0], { exact: false }),
    fixture("homePullStart"),
    wait("離して更新"),
    fixture("homePullFinish"),
    wait("引っ張って最新の投稿を取得しました"),
    role("status", undefined, { state: "hidden" }),
  ],
  "user-search-empty": [...search("nobody"), wait("ユーザーが見つかりません")],
  "user-search-error": [...search("@bob@social.example"), role("alert")],
  "timeline-switch": ["ホーム", "ローカル", "ソーシャル", "ホーム"].flatMap((name) => [
    button(name),
    wait(`${name}の投稿`),
    role("button", name, { pressed: true }),
  ]),
  like: [
    ...heart,
    role("button", "❤️ 1", { pressed: true }),
    button("❤️ 1"),
    role("button", "❤️ 1", { state: "hidden" }),
    role("button", "リアクション 0"),
  ],
  "like-error": [...heart, role("alert")],
  "action-bar": [
    role("group", "投稿のアクション"),
    fixture("homeActionBar"),
    button("投稿の操作", { within: { role: "group", name: "投稿のアクション" } }),
    role("menuitem", "投稿を編集"),
  ],
  reply: [...reply, type("返信のメッセージ"), send, closed, wait("返信のメッセージ")],
  "mobile-reply": [
    fixture("homeObserveComposerFocus"),
    ...reply,
    fixture("homeComposerFocused"),
    type("返信のメッセージ"),
    send,
    closed,
    wait("返信のメッセージ"),
  ],
  "private-reply": [
    role("button", "リポスト 0", { enabled: false }),
    button("返信 0"),
    role("dialog", "近況を投稿"),
    button("公開範囲: フォロワー限定"),
    role("menuitemradio", "パブリック", { enabled: false }),
    role("menuitemradio", "ホーム", { enabled: false }),
  ],
  repost: [
    button("リポスト 0"),
    wait("Aliceさんがリポスト"),
    role("button", "リポストを取り消す 1", { count: 2 }),
    button("リポストを取り消す 1", { index: 0 }),
    wait("Aliceさんがリポスト", { state: "hidden" }),
  ],
  "remote-avatar": [
    role("img", "Remote Alice", { busy: false }),
    wait("RA", { state: "hidden" }),
    fixture("homeAvatarLinks"),
  ],
  "remote-avatar-fallback": [role("img", "Remote Alice", { busy: false }), wait("RA")],
  "single-post": [
    wait(homePosts[0].content.split("\n")[0], { exact: false }),
    role("article", undefined, { count: 1 }),
    role("button", "ローカル", { state: "hidden" }),
  ],
  "single-post-missing": [role("alert"), role("article", undefined, { count: 0 })],
  "user-profile": [wait("リモートユーザーの自己紹介です"), button("フォロー"), wait("承認待ち")],
  "mobile-user-profile": [
    wait("リモートユーザーの自己紹介です"),
    button("フォロー"),
    wait("承認待ち"),
  ],
  "user-profile-error": [role("alert")],
  "misskey-custom-emoji": [wait("@misskey.example", { exact: false }), fixture("homeEmojiImages")],
  "misskey-react": [
    button("リアクション 2"),
    role("dialog", "リアクション"),
    button(":blobcat:", { within: { role: "dialog", name: "リアクション" } }),
    role("button", ":blobcat: 3", { pressed: true }),
    button(":blobcat: 3"),
    role("button", ":blobcat: 2", { pressed: false }),
  ],
  "mobile-misskey": [wait("@misskey.example", { exact: false }), fixture("homeEmojiImages")],
  "emoji-fallback": [fixture("homeEmojiFallback")],
  "live-while-loading-older": [
    fixture("homeOlderLiveReady"),
    { action: "scroll", role: "feed", deltaY: 100000 },
    button("過去の投稿を読む"),
    fixture("homeOlderRefresh"),
    wait("更新と競合しても残る過去の投稿"),
    fixture("homeOlderRetained"),
    wait("更新と競合しても残る過去の投稿"),
  ],
};

export function homeOptions(
  context: Selection,
  base: KotlinPreviewProps,
): KotlinPreviewProps | null {
  if (context.title !== "Pages/Home") return null;
  const id = context.id.startsWith("pages-home--")
    ? context.id.slice("pages-home--".length)
    : context.name
        .replace(/([a-z0-9])([A-Z])/g, "$1-$2")
        .replace(/\s+/g, "-")
        .toLowerCase();
  const interactions = homeInteractions[id];
  if (!interactions) throw new Error(`Unmapped Home Story: ${context.id}`);
  return {
    ...base,
    sample: "home-story",
    fullHeight: true,
    surfaceClassName: "h-[720px] min-w-0 bg-background",
    fixture: {
      ...base.fixture,
      sharedApi: true,
      storyTitle: context.title,
      state: context.name,
      args: context.args,
      interactions,
    },
  };
}
