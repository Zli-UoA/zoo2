import { createStoryConfig, type StoryCollectionsInput } from "./fixtures";
import { notificationComparisonText, notificationComparisonLabel } from "./notification-fixtures";
import type { KotlinPreviewProps } from "./KotlinPreview";
import type { NativeInteraction } from "./native-interactions";
import type { Selection } from "./kotlin-stories";

function record(value: unknown): Record<string, unknown> {
  return value !== null && typeof value === "object" ? (value as Record<string, unknown>) : {};
}
// These operations are the Kotlin counterpart of each original CSF play. They
// query actual rendered semantics; advanced Canvas operations are added below
// rather than treating catalog readiness as a successful interaction.
function notificationInteractions(title: string, name: string): NativeInteraction[] {
  const visible = (text: string): NativeInteraction => ({
    action: "wait",
    text,
    exact: false,
    all: true,
  });
  const absent = (text: string): NativeInteraction => ({
    action: "wait",
    text,
    exact: false,
    state: "hidden",
  });
  const ready: NativeInteraction[] = [
    { action: "wait", role: "region", name: "通知" },
    { action: "wait", role: "list", name: "通知" },
    { action: "wait", role: "status", state: "hidden" },
  ];
  if (title === "Pages/Notifications") {
    if (name === "Loading")
      return [
        { action: "wait", role: "region", name: "通知" },
        { action: "wait", role: "status" },
        { action: "wait", role: "list", name: "通知", state: "hidden" },
        { action: "wait", text: "未読", count: 3, state: "present" },
      ];
    if (name === "Direct Messages")
      return [...ready, visible("Admin, Bob, Carol +1"), visible("Bob"), absent("#dm:")];
    if (name === "Bodies Ready Together")
      return [
        { action: "wait", role: "region", name: "通知" },
        { action: "wait", role: "status" },
        { action: "wait", role: "list", name: "通知", state: "hidden" },
        { action: "fixture", name: "releaseBodies" },
        ...ready,
        visible("本文 notification-mention"),
        { action: "wait", text: "本文 notification-", exact: false, count: 4 },
      ];
    if (name.startsWith("Message Layout Comparison"))
      return [
        ...ready,
        { action: "wait", role: "region", name: "メッセージ表示の比較用" },
        { action: "wait", role: "heading", name: "通知" },
        {
          action: "wait",
          role: "heading",
          name: "general",
          exact: false,
          sameHeightAs: { role: "heading", name: "通知" },
        },
        visible("今日のリリースは 18:00"),
        { action: "overflow" },
      ];
    if (name === "Loading On Scroll")
      return [
        ...ready,
        visible("本文 scroll-0"),
        { action: "scroll", role: "list", name: "通知", deltaY: 100000 },
        { action: "wait", role: "status", containedBy: { role: "region", name: "通知" } },
        { action: "wait", text: "本文 scroll-0", state: "present" },
        {
          action: "wait",
          role: "list",
          name: "通知",
          attributes: { inert: null, "aria-hidden": null },
        },
        { action: "scroll", role: "list", name: "通知", deltaY: -100000 },
        ...ready,
        visible("本文 scroll-0"),
      ];
    if (["Default", "Mobile", "Dark"].includes(name))
      return [
        ...ready,
        visible("今日のリリースは 18:00"),
        {
          action: "wait",
          role: "list",
          name: "通知",
          sameWidthAs: { role: "region", name: "通知" },
        },
        {
          action: "wait",
          role: "listitem",
          within: { role: "list", name: "通知" },
          directWithin: true,
          all: true,
          sameWidthAs: { role: "list", name: "通知" },
        },
        {
          action: "wait",
          role: "link",
          name: "example.com/docs",
          attributes: { target: "_blank" },
        },
      ];
    return ready;
  }
  if (title !== "Features/Notifications/Row") return [];
  switch (name) {
    case "Mention":
      return [visible("今日のリリースは 18:00")];
    case "Direct Message":
      return [
        visible("Bob"),
        absent("#dm:"),
        {
          action: "wait",
          role: "link",
          name: "個別に確認",
          exact: false,
          attributeIncludes: { href: "/channels/channel-dm?" },
        },
      ];
    case "Group Direct Message":
      return [visible("Admin, Bob, Carol +1"), absent("dm:group:")];
    case "Dm Metadata Pending":
      return [visible("ダイレクトメッセージ"), absent("#dm:")];
    case "Grouped Reactions":
      return [
        visible("Bob さんほか1人"),
        { action: "wait", role: "listitem", name: "👀 2" },
        {
          action: "wait",
          role: "link",
          name: "example.com/release",
          attributes: { target: "_blank" },
        },
      ];
    case "Loading":
      return [visible("読み込み中…")];
    case "Unavailable":
      return [
        visible("メッセージが削除されたか、閲覧できません"),
        { action: "wait", role: "link", count: 0 },
      ];
    case "Long Content Mobile":
      return [visible("確認をお願いします"), { action: "overflow" }];
    case "Native Interactions":
      return [
        {
          action: "wait",
          role: "link",
          name: "example.com/docs",
          attributes: { target: "_blank" },
        },
        { action: "wait", role: "link", name: "メンション · #general", exact: false },
        { action: "selection", text: "今日のリリースは 18:00", exact: false, selection: "set" },
        { action: "pointer", text: "今日のリリースは 18:00", exact: false },
        { action: "navigation", count: 0 },
        { action: "selection", selection: "clear" },
        { action: "pointer", text: "今日のリリースは 18:00", exact: false },
        {
          action: "navigation",
          count: 1,
          urlIncludes: "/channels/channel-general?",
          newTab: false,
        },
        { action: "pointer", text: "今日のリリースは 18:00", exact: false, ctrlKey: true },
        { action: "navigation", count: 2, urlIncludes: "/channels/channel-general?", newTab: true },
      ];
    default:
      return [];
  }
}

export function notificationOptions(
  context: Selection,
  base: KotlinPreviewProps,
): KotlinPreviewProps | null {
  const { title, name, args, parameters } = context;
  const fixture: Record<string, unknown> = {
    ...base.fixture,
    state: name,
    interactions: notificationInteractions(title, name),
  };
  if (title === "Pages/Notifications")
    return {
      ...base,
      sample: "notification-story",
      fullHeight: true,
      surfaceClassName: "h-[720px] overflow-hidden bg-background",
      fixture: {
        ...fixture,
        view: "screen",
        comparisonText: notificationComparisonText,
        comparisonLabel: notificationComparisonLabel,
        ...([
          "Default",
          "Mobile",
          "Direct Messages",
          "Dark",
          "Message Layout Comparison",
          "Message Layout Comparison Mobile",
          "Loading On Scroll",
        ].includes(name)
          ? { focused: false }
          : {}),
        comparison: name.startsWith("Message Layout Comparison"),
      },
    };
  if (title === "Features/Notifications/Row") {
    const group = record(args.group);
    return {
      ...base,
      sample: "notification-story",
      fullHeight: false,
      fixture: {
        ...fixture,
        view: "row",
        notifications: group.notifications ?? [],
        channel: args.channel ?? null,
        members: args.members ?? [],
        users: args.users instanceof Map ? [...args.users.values()] : [],
        readEnabled: args.readEnabled ?? true,
      },
    };
  }
  if (title === "Features/Notifications/Button")
    return {
      ...base,
      sample: "notification-story",
      fullHeight: false,
      measureWidth: true,
      surfaceClassName: "flex items-center bg-background p-6",
      fixture: { ...fixture, view: "button" },
    };
  if (title === "Features/Status/Sync")
    return {
      ...base,
      sample: "notification-story",
      measureWidth: true,
      surfaceClassName: "w-fit bg-background p-4",
      fixture: {
        ...fixture,
        view: "sync",
        config: createStoryConfig(parameters.collections as StoryCollectionsInput | undefined),
      },
    };
  if (title === "Features/Status/Offline") {
    const offline = record(parameters.offline);
    return {
      ...base,
      sample: "notification-story",
      fullHeight: true,
      surfaceClassName: "h-[192px] bg-background",
      callbacks:
        typeof offline.retryPersistence === "function"
          ? { retryPersistence: offline.retryPersistence as (...args: unknown[]) => unknown }
          : {},
      fixture: {
        ...fixture,
        view: "offline",
        callbacks: [...((fixture.callbacks as string[] | undefined) ?? []), "retryPersistence"],
        offline: {
          isOnline: offline.isOnline ?? true,
          persistenceStatus: offline.persistenceStatus ?? "ready",
          persistenceError: offline.persistenceError ?? "",
        },
      },
    };
  }
  if (title === "Pages/Skeleton")
    return {
      ...base,
      sample: "notification-story",
      fullHeight: true,
      fixture: { ...fixture, view: "skeleton" },
    };
  return null;
}
