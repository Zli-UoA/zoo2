import { storyDeploymentCard } from "./app-fixtures";
import kitchenCatalogUrl from "../src/lib/emoji/emoji-kitchen.json?url";
import { githubFeedCards, reactionKitchenCatalog } from "./message-fixtures";
import type { KotlinPreviewProps } from "./KotlinPreview";
import type { Selection } from "./kotlin-stories";

const titles = new Set(
  [
    "MessageRow",
    "Composer",
    "MessageContent",
    "MessageCard",
    "MessageList",
    "MessagePane",
    "ThreadPane",
    "ReactionPicker",
    "ReactionEmoji",
    "LinkPreviews",
    "Typing",
  ].map((name) => `Features/Chat/${name}`),
);
function data(value: unknown): unknown {
  if (value instanceof Map)
    return Object.fromEntries([...value].map(([key, item]) => [key, data(item)]));
  if (value instanceof Set) return [...value].map(data);
  if (Array.isArray(value)) return value.map(data);
  if (value && typeof value === "object")
    return Object.fromEntries(
      Object.entries(value)
        .filter(([, item]) => typeof item !== "function" && typeof item !== "symbol")
        .map(([key, item]) => [key, data(item)]),
    );
  return typeof value === "function" || typeof value === "symbol" ? undefined : value;
}
export function messageOptions(
  context: Selection,
  base: KotlinPreviewProps,
): KotlinPreviewProps | null {
  if (!titles.has(context.title)) return null;
  const component = context.title.split("/").at(-1)!;
  const collections = context.parameters.collections ?? {};
  const item = context.args.item as { message?: Record<string, unknown> } | undefined;
  const messages = (context.args.messages ??
    context.args.replies ??
    (item?.message ? [item.message] : [])) as Record<string, unknown>[];
  const groupSets =
    context.args.reactions instanceof Map
      ? [...context.args.reactions.entries()]
      : [[item?.message?.id, context.args.reactions]];
  const reactions: Record<string, unknown>[] =
    context.args.reactions === undefined
      ? [...((base.fixture?.reactions as Record<string, unknown>[]) ?? [])]
      : [];
  const actors = new Map(
    ((base.fixture?.users as Record<string, unknown>[]) ?? []).map((user) => [user.id, user]),
  );
  for (const message of messages)
    actors.set(message.user_id, {
      id: message.user_id,
      display_name: message.display_name,
      user_name: message.user_name,
    });
  for (const [messageId, groups] of groupSets) {
    if (!Array.isArray(groups)) continue;
    for (const group of groups)
      for (const [index, userId] of (group.userIds ?? []).entries()) {
        reactions.push({
          id:
            group.ownId && userId === base.fixture?.currentUserId
              ? group.ownId
              : `${messageId}-${group.emoji}-${index}`,
          message_id: messageId,
          user_id: userId,
          emoji: group.emoji,
        });
        const label = group.userNames?.[userId];
        if (typeof label === "string") {
          const match = /^(.*?) \(@(.*?)\)$/.exec(label);
          actors.set(userId, {
            id: userId,
            display_name: match?.[1] ?? "",
            user_name: match?.[2] ?? label.replace(/^@/, ""),
          });
        }
      }
  }
  return {
    ...base,
    callbacks: Object.fromEntries(
      Object.entries(context.args).filter(([, value]) => typeof value === "function"),
    ) as KotlinPreviewProps["callbacks"],
    sample: "message-story" as KotlinPreviewProps["sample"],
    fullHeight: ["MessageList", "MessagePane", "ThreadPane"].includes(component),
    measureWidth: ["ReactionEmoji", "ReactionPicker"].includes(component),
    fixture: {
      ...base.fixture,
      ...(data(collections) as Record<string, unknown>),
      component,
      kitchenCatalogUrl,
      feed: context.name === "Git Hub Feed" ? githubFeedCards : undefined,
      state: context.name,
      args: data(
        component === "ReactionEmoji" && context.name === "Consistent Size"
          ? { ...context.args, catalog: reactionKitchenCatalog }
          : context.args,
      ),
      callbacks: Object.entries(context.args)
        .filter(([, value]) => typeof value === "function")
        .map(([key]) => key),
      interactions: messageInteractions(component, context.name),
      reactions,
      users: [...actors.values()],
    },
    surfaceClassName: messageSurfaces[component] ?? base.surfaceClassName,
  };
}

function messageInteractions(component: string, state: string): Record<string, unknown>[] {
  const click = (role: string, name: string) => ({ action: "click", role, name, exact: true });
  const wait = (role: string, name?: string, extra: Record<string, unknown> = {}) => ({
    action: "wait",
    role,
    ...(name === undefined ? {} : { name }),
    ...extra,
  });
  if (component === "MessagePane" && state === "Default")
    return [
      { action: "wait", text: "今日のリリースは 18:00", exact: false },
      wait("heading", "メッセージ", { state: "hidden" }),
    ];
  if (component === "ThreadPane" && state === "App Card")
    return [
      wait("link", storyDeploymentCard.title),
      { action: "wait", text: "Plain fallback for card-capable clients", state: "hidden" },
    ];
  if (component === "ReactionEmoji" && state === "Consistent Size")
    return [wait("img", undefined, { width: 20, height: 20 })];
  if (component === "ReactionPicker" && state === "Default")
    return [
      wait("dialog"),
      wait("row", undefined, {
        all: true,
        minAttributes: { "aria-rowindex": 1 },
        within: { role: "dialog" },
      }),
      wait("gridcell", undefined, {
        all: true,
        minAttributes: { "aria-colindex": 1 },
        within: { role: "dialog" },
      }),
    ];
  if (component === "MessageContent" && state === "Headings")
    return [
      ...Array.from({ length: 6 }, (_, index) =>
        wait("heading", undefined, { index, attributes: { "aria-level": String(index + 1) } }),
      ),
      wait("heading", undefined, { count: 6, all: true }),
      wait("link", "ドキュメント", { attributes: { href: "https://example.com/docs" } }),
      { action: "wait", text: "# コード内はそのまま" },
      { action: "wait", text: "## 装飾しない", exact: false },
    ];
  if (component === "MessageList" && state === "Unread Separator")
    return [{ action: "wait", role: "separator", name: "新着メッセージ" }];
  if (component === "MessageList" && state === "Delayed Link Preview")
    return [{ action: "wait", role: "link", name: "Delayed URL card", exact: false, busy: false }];
  if (component === "LinkPreviews")
    return [
      {
        action: "wait",
        role: "link",
        name: "Zooのリリース情報",
        exact: false,
        busy: false,
        maxWidth: 384,
      },
    ];
  if (component === "MessageCard" && state === "Collapsible Body")
    return [
      wait("button", "もっと見る", { attributes: { "aria-expanded": "false" } }),
      { action: "wait", text: "Completed" },
      { action: "wait", text: "Deployment bot" },
      click("button", "もっと見る"),
      wait("button", "折りたたむ", { attributes: { "aria-expanded": "true" } }),
      click("button", "折りたたむ"),
      wait("button", "もっと見る", { attributes: { "aria-expanded": "false" } }),
    ];
  if (component === "MessagePane" && state === "Thread Open")
    return [click("button", "2 件の返信（スレッドを開く）"), wait("complementary", "スレッド")];
  if (component === "ThreadPane" && state === "Completions")
    return [
      { action: "type", role: "textbox", text: "#gen" },
      click("option", "#general"),
      wait("textbox", undefined, { valueExact: "#general " }),
      { action: "type", role: "textbox", value: "/git" },
      { action: "key", key: "Enter" },
      wait("textbox", undefined, { valueExact: "/github " }),
      click("button", "送信"),
      { action: "callback", name: "onSendReply", args: ["/github", "github"] },
    ];
  if (component === "MessageRow") {
    if (state === "Edited")
      return [
        { action: "hover", role: "img", name: "編集済み", exact: true },
        wait("tooltip", undefined, { value: "2026" }),
        wait("link", "リンク", { attributes: { href: "https://example.com/guide" } }),
      ];
    if (state === "Hover Actions")
      return [{ action: "hover", text: "こんにちは" }, wait("button", "編集")];
    if (state === "Reaction Users")
      return [
        click("button", "👍 にリアクションしたユーザーを表示"),
        { action: "wait", role: "dialog" },
        { action: "wait", text: "@longname", exact: false, within: { role: "dialog" } },
        { action: "wait", text: "@bob", exact: false, within: { role: "dialog" } },
      ];
    if (state === "Reaction Users Hover")
      return [
        { action: "hover", role: "button", name: "👍 3 件", exact: false },
        wait("tooltip", undefined, { value: "@alice" }),
      ];
    if (state === "Copy Original Menu")
      return [
        { action: "focus", role: "button", name: "その他の操作" },
        { action: "key", key: "Enter" },
        { action: "wait", role: "menuitem", name: "原文をコピー" },
      ];
    if (state === "With Attachments")
      return [
        click("button", "screenshot.pngをプレビュー"),
        { action: "wait", role: "img", name: "screenshot.png", within: { role: "dialog" } },
        { action: "click", role: "button", name: "閉じる", within: { role: "dialog" } },
        { action: "wait", role: "dialog", state: "hidden" },
      ];
    if (state === "Reply Count")
      return [{ action: "wait", role: "button", name: "3 件の返信（スレッドを開く）" }];
    if (state === "App Message") return [{ action: "wait", role: "button", name: "詳細を確認" }];
  }
  if (component === "Composer") {
    if (state === "Disabled") return [wait("textbox", undefined, { enabled: false })];
    if (state === "Multiline")
      return [{ action: "type", role: "textbox", text: "1行目\n2行目\n3行目" }];
    if (state === "Mention Suggestions")
      return [{ action: "type", role: "textbox", text: "@al" }, wait("listbox", "メンション候補")];
    if (state === "Channel Suggestions")
      return [
        { action: "type", role: "textbox", text: "詳しくは #gen" },
        wait("listbox", "チャンネル候補"),
        wait("option", "#general"),
      ];
    if (state === "Channel Insertion")
      return [
        { action: "type", role: "textbox", text: "#gen" },
        { action: "key", key: "Tab" },
        wait("textbox", undefined, { valueExact: "#general " }),
        wait("listbox", undefined, { state: "hidden" }),
      ];
    if (state === "Send Error")
      return [
        { action: "type", role: "textbox", text: "Catalog send failure" },
        click("button", "送信"),
        { action: "wait", role: "textbox", value: "Catalog send failure" },
      ];
    if (state === "Command Suggestions")
      return [
        { action: "type", role: "textbox", text: "/ec" },
        wait("listbox", "コマンド候補"),
        click("option", "/echo Meeting Notes"),
        wait("textbox", undefined, { valueExact: "/echo " }),
        { action: "type", role: "textbox", text: "hello" },
        click("button", "送信"),
        { action: "callback", name: "onSend", args: ["/echo hello", "notes"] },
      ];
    if (["With Attachments", "With File Attachment", "Attachment Upload Error"].includes(state))
      return [
        {
          action: "paste",
          role: "textbox",
          file:
            state === "With File Attachment"
              ? { name: "release-notes.pdf", type: "application/pdf", base64: "AQIDBAU=" }
              : { name: "screenshot.png", type: "image/png", base64: "AQIDBA==" },
        },
        ...(state === "Attachment Upload Error"
          ? [
              wait("alert", undefined, { value: "ファイルをアップロードできませんでした。" }),
              wait("img", "screenshot.png", { state: "hidden" }),
            ]
          : [
              wait(
                "button",
                state === "With File Attachment"
                  ? "release-notes.pdf を選択から外す"
                  : "screenshot.png を選択から外す",
              ),
              { action: "wait", role: "button", name: "送信", enabled: true },
              click("button", "送信"),
              {
                action: "callback",
                name: "onSend",
                args: [
                  "",
                  { __zooUndefined: true },
                  [state === "With File Attachment" ? "attachment-2" : "attachment-1"],
                ],
              },
            ]),
      ];
  }
  if (component === "ReactionPicker") {
    if (state === "Kitchen" || state === "Kitchen Loading")
      return [
        click("tab", "Emoji Kitchen"),
        ...(state === "Kitchen"
          ? [{ action: "wait", role: "combobox", name: "1つ目の絵文字" }]
          : []),
      ];
    if (state === "Search Empty")
      return [
        { action: "type", role: "textbox", name: "絵文字を検索", text: "zzzzzzzz" },
        { action: "wait", text: "一致する絵文字がありません" },
      ];
  }
  return [];
}
export const messageSurfaces: Record<string, string> = {
  MessageRow: "mx-auto max-w-3xl bg-background py-8",
  Composer: "mx-auto max-w-3xl bg-background",
  MessageContent:
    "mx-auto max-w-2xl whitespace-pre-wrap break-words bg-background p-4 text-sm text-foreground",
  MessageCard: "mx-auto max-w-3xl bg-background p-6",
  MessageList: "mx-auto flex h-[640px] max-w-3xl flex-col bg-background",
  MessagePane: "flex h-[720px] min-w-0 flex-1 bg-background",
  ThreadPane: "flex h-[640px] justify-end bg-muted/30",
  LinkPreviews: "mx-auto max-w-4xl p-4",
  ReactionPicker: "flex min-h-96 items-center justify-center bg-background p-8",
  Typing: "bg-background py-4",
};
