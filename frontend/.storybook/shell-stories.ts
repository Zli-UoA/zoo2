import type { KotlinPreviewProps } from "./KotlinPreview";
import type { NativeInteraction } from "./native-interactions";
import type { Selection } from "./kotlin-stories";

const titles = new Set([
  "Features/Channels/Discovery",
  "Features/Channels/Header",
  "Features/Channels/Sidebar",
  "Features/Channels/Create",
  "Features/Channels/Edit",
  "Features/Channels/Members",
  "Features/DMs/Create",
  "Features/DMs/ListItem",
  "Features/DMs/Sidebar",
  "Features/Navigation/Rail",
  "Pages/Channel",
  "Pages/ChannelMembers",
  "Pages/Welcome",
  "Pages/Skeleton",
  "Pages/Shell",
  "Pages/BubbleChat",
]);
const surfaces: Record<string, string> = {
  "Features/Channels/Sidebar": "h-[640px] bg-background",
  "Features/Navigation/Rail": "h-[640px] bg-background",
  "Features/DMs/Sidebar": "w-72 bg-sidebar",
  "Features/DMs/ListItem": "w-72 bg-sidebar p-2",
  "Features/Channels/Members": "h-[640px] w-80 bg-background",
  "Pages/Channel": "h-[720px] min-w-0 bg-background",
  "Pages/ChannelMembers": "h-dvh bg-background",
  "Pages/Welcome": "h-[640px] bg-background",
  "Pages/BubbleChat": "h-[720px] overflow-hidden bg-background",
};
function row(value: unknown): Record<string, unknown> {
  return value !== null && typeof value === "object" ? (value as Record<string, unknown>) : {};
}
export function shellOptions(
  context: Selection,
  base: KotlinPreviewProps,
): KotlinPreviewProps | null {
  const { title, name, args, parameters } = context;
  if (!titles.has(title)) return null;
  const fixture = { ...base.fixture };
  for (const field of [
    "channels",
    "users",
    "memberships",
    "currentUserId",
    "channel",
    "channelId",
    "label",
    "active",
    "unread",
    "loading",
    "error",
    "usersError",
    "activeChannelId",
    "memberCount",
    "subtitle",
    "open",
  ])
    if (args[field] !== undefined) fixture[field] = args[field];
  if (args.unreadChannelIds instanceof Set) fixture.unreadIds = [...args.unreadChannelIds];
  if (args.joinedChannelIds instanceof Set) fixture.joinedIds = [...args.joinedChannelIds];
  if (typeof args.title === "string") fixture.displayTitle = args.title;
  fixture.title = title;
  fixture.state = name;
  const interactions: NativeInteraction[] = [];
  const click = (role: string, label: string): NativeInteraction => ({
    action: "click",
    role,
    name: label,
  });
  const wait = (
    role: string,
    label: string,
    state: "visible" | "hidden" = "visible",
  ): NativeInteraction => ({ action: "wait", role, name: label, state });
  if (title === "Features/Channels/Create" && name === "Filled")
    interactions.push(
      { action: "type", role: "textbox", name: "新しいチャンネル名", text: "new-channel" },
      { action: "type", role: "textbox", name: "説明（任意）", text: "新しいチャンネルの説明" },
      click("checkbox", "非公開にする"),
      { ...wait("button", "チャンネル作成"), enabled: true },
    );
  if (title === "Features/Channels/Edit" && name === "Filled") {
    for (const [label, text] of [
      ["新しいチャンネル名", "general-renamed"],
      ["説明（任意）", "チーム全体の連絡用チャンネル"],
    ])
      interactions.push(
        { action: "focus", role: "textbox", name: label },
        { action: "click", role: "textbox", name: label },
        { action: "key", key: "ControlOrMeta+A" },
        { action: "key", key: "Backspace" },
        { action: "type", role: "textbox", name: label, text },
      );
    interactions.push({ ...wait("button", "保存"), enabled: true });
  }
  if (title === "Features/Navigation/Rail" && ["Default", "Mobile"].includes(name))
    interactions.push(wait("button", "Compose版を使う（実験・開発中）"));
  if (title === "Features/Channels/Discovery" && name === "Default")
    interactions.push({ ...wait("link", "random"), exact: false });
  if (title === "Features/Channels/Discovery" && name === "Search Empty")
    interactions.push(
      { action: "type", role: "searchbox", text: "zzzz" },
      { action: "wait", text: "一致するチャンネルがありません。" },
    );
  if (
    title === "Features/Channels/Header" &&
    ["Control Panel Open", "Webhook Settings Open"].includes(name)
  ) {
    interactions.push(click("button", "チャンネル設定"), wait("dialog", "チャンネル設定"));
    if (name === "Webhook Settings Open")
      interactions.push(
        { ...click("button", "開く"), within: { role: "dialog", name: "チャンネル設定" } },
        wait("dialog", "Webhook設定"),
        wait("dialog", "チャンネル設定", "hidden"),
        { action: "key", key: "Escape" },
        wait("dialog", "Webhook設定", "hidden"),
      );
  }
  if (title === "Features/Channels/Members" && name === "Add Candidates")
    interactions.push(click("combobox", "メンバーを追加"), wait("listbox", "メンバー候補"));
  if (title === "Features/Channels/Sidebar" && ["Sections", "Mobile Sections"].includes(name))
    interactions.push(
      wait("link", "general"),
      wait("link", "Bob"),
      click("button", "チャンネル"),
      wait("link", "general", "hidden"),
      wait("link", "Bob"),
      { action: "key", key: "Enter" },
      wait("link", "general"),
      click("button", "ダイレクトメッセージ"),
      wait("link", "Bob", "hidden"),
      wait("link", "general"),
      click("button", "ダイレクトメッセージ"),
      wait("link", "Bob"),
    );
  if (title === "Features/DMs/Create" && name === "Selected")
    interactions.push(
      { action: "type", role: "combobox", text: "Bob" },
      { action: "click", role: "option", name: "Bob", exact: false },
      { action: "wait", role: "button", name: "Bob", exact: false },
    );
  if (title === "Features/DMs/Create" && name === "Search Empty")
    interactions.push(
      { action: "type", role: "combobox", text: "no-such-user" },
      { action: "wait", role: "option", state: "hidden" },
    );
  if (title === "Pages/Channel" && name === "Command Completion")
    interactions.push(
      { action: "type", role: "textbox", text: "/ec" },
      click("option", "/echo Meeting Notes"),
      { action: "type", role: "textbox", text: "hello" },
      click("button", "送信"),
      { action: "wait", role: "textbox", value: "" },
    );
  if (title === "Pages/ChannelMembers" && name === "Default")
    interactions.push(wait("heading", "メンバー"), wait("button", "チャンネルに戻る"), {
      action: "wait",
      role: "dialog",
      state: "hidden",
    });
  if (title === "Features/DMs/Sidebar" && name === "Empty")
    interactions.push(
      wait("button", "DMを始める", "hidden"),
      wait("button", "ダイレクトメッセージ"),
    );
  if (title === "Features/Channels/Sidebar" && name === "Unread And Archived")
    interactions.push(
      { ...wait("img", "未読メッセージあり"), index: 0 },
      { ...wait("img", "未読メッセージあり"), index: 1 },
      wait("img", "アーカイブ済み"),
    );
  if (title === "Pages/BubbleChat") {
    if (name === "Empty")
      interactions.push({ action: "wait", text: "現在入力中のユーザーはいません" });
    if (name === "Active Conversation")
      interactions.push(
        { action: "wait", text: "今日のミーティングは何時からでしたっけ？" },
        { action: "wait", text: "15時からだった気がします！" },
        {
          action: "type",
          role: "textbox",
          name: "リアルタイムメッセージ入力",
          text: "ありがとう！準備してきます",
        },
        { action: "wait", text: "ありがとう！準備してきます", within: { role: "list" } },
        { action: "key", key: "Control+Enter" },
        { action: "wait", role: "textbox", name: "リアルタイムメッセージ入力", value: "" },
      );
    if (name.includes("Participants"))
      interactions.push(
        { action: "wait", text: "今日のミーティングは何時からでしたっけ？" },
        { action: "click", role: "button", name: "参加者 ", exact: false },
        {
          action: "wait",
          text: "@bob",
          within:
            name === "Mobile Participants"
              ? { role: "dialog" }
              : { role: "complementary", name: "参加者" },
        },
      );
  }
  fixture.interactions = interactions;

  let path = (parameters.router?.initialEntries as string[] | undefined)?.[0] ?? "/";
  if (title === "Pages/Channel")
    path = `/channels/${typeof args.channelId === "string" ? args.channelId : "channel-general"}`;
  if (title === "Pages/BubbleChat") path = "/bubble";
  if (title === "Pages/Shell") {
    const child = row(row(args.children).props);
    if (typeof child.channelId === "string" && path === "/") path = `/channels/${child.channelId}`;
    if (name.includes("Notifications")) path = "/notifications";
    if (name.includes("Bubble Chat")) path = "/bubble";
    fixture.membersOpen =
      new URL(path, "http://storybook.local").searchParams.get("members") === "true";
    if (name === "Welcome") interactions.push(wait("complementary", "チャンネル"));
    if (["Channel", "Channel With Members"].includes(name))
      interactions.push(
        { action: "wait", text: "今日のリリースは 18:00", exact: false },
        wait("complementary", "チャンネル"),
      );
    if (name === "Mobile Channel") interactions.push(wait("heading", "general"));
    if (["Notifications", "Mobile Notifications", "Dark Notifications"].includes(name))
      interactions.push(
        { action: "wait", role: "status", state: "hidden" },
        wait("complementary", "チャンネル", "hidden"),
      );
    if (["Bubble Chat", "Mobile Bubble Chat"].includes(name))
      interactions.push(
        wait("textbox", "リアルタイムメッセージ入力"),
        wait("complementary", "チャンネル", "hidden"),
        wait("link", "Bubble Chat"),
      );
    if (name === "Mobile Members")
      interactions.push(
        wait("heading", "メンバー"),
        { action: "wait", role: "dialog", state: "hidden" },
        wait("navigation", "メインナビゲーション", "hidden"),
      );
    if (name === "Channel With Members")
      interactions.push(
        { action: "click", role: "button", name: "メンバー ", exact: false },
        wait("heading", "メンバー"),
      );
  }
  return {
    ...base,
    sample: "shell-story",
    path,
    fullHeight: ![
      "Features/Channels/Create",
      "Features/Channels/Edit",
      "Features/Channels/Discovery",
      "Features/DMs/Create",
      "Features/DMs/ListItem",
      "Features/DMs/Sidebar",
      "Features/Channels/Header",
    ].includes(title),
    surfaceClassName: surfaces[title],
    fixture,
  };
}
