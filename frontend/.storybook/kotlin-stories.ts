import { avatarStoryInitials } from "./basic-fixtures";
import { primitiveOptions } from "./primitive-stories";
import { messageOptions } from "./message-stories";
import { shellOptions } from "./shell-stories";
import { appOptions } from "./app-stories";
import { preferenceOptions } from "./preference-stories";
import { notificationOptions } from "./notification-stories";
import { collectStoryCallbacks } from "./story-callbacks";
import { dialogSamples } from "./dialog-fixtures";
import type { StoryContext } from "@storybook/react-vite";
import type { KotlinPreviewProps } from "./KotlinPreview";
import { isLocale } from "../src/lib/i18n/locale";
import { storySurfaces } from "./story-surfaces";
import { storyCollections } from "./fixtures";

export type Selection = Pick<
  StoryContext,
  "id" | "title" | "name" | "args" | "parameters" | "globals"
>;

/** Map existing stories to real Kotlin components; never substitute React for a missing mapping. */
function record(value: unknown): Record<string, unknown> {
  return value !== null && typeof value === "object" ? (value as Record<string, unknown>) : {};
}

export function kotlinOptions(context: Selection): KotlinPreviewProps | null {
  const { args, title, name, parameters, globals } = context;
  const collections = parameters.collections ?? storyCollections;
  const fixture: Record<string, unknown> = {
    ...storyCollections,
    ...collections,
    callbacks: Object.keys(collectStoryCallbacks(args)),
    sharedApi: true,
    currentUserId: collections.currentUserId ?? storyCollections.currentUserId,
    isAdmin: title === "Pages/ZooSettings" && name === "Members",
    users: collections.users ?? storyCollections.users,
    channels: collections.channels ?? storyCollections.channels,
    notifications: collections.notifications ?? storyCollections.notifications,
  };
  const base: KotlinPreviewProps = {
    surfaceClassName: storySurfaces[title as keyof typeof storySurfaces],
    fullHeight: title.startsWith("Pages/"),
    sample: "message",
    content: "",
    disabled: Boolean(args.disabled),
    grouped: false,
    locale: isLocale(globals.locale) ? globals.locale : "ja",
    dark: globals.theme === "dark",
    fixture,
  };
  for (const adapter of [
    primitiveOptions,
    messageOptions,
    shellOptions,
    appOptions,
    preferenceOptions,
    notificationOptions,
  ]) {
    const result = adapter(context, base);
    if (result) return result;
  }
  if (["Components/Button", "Components/Input", "Components/Textarea"].includes(title)) {
    function text(value: unknown): string {
      if (typeof value === "string" || typeof value === "number") return String(value);
      if (Array.isArray(value)) return value.map(text).join("");
      return value && typeof value === "object" ? text(record(record(value).props).children) : "";
    }
    return {
      ...base,
      sample:
        title === "Components/Button"
          ? "button"
          : title === "Components/Input"
            ? "input"
            : "textarea",
      fixture: {
        ...fixture,
        state: name,
        label: text(args.children),
        variant: args.variant ?? "default",
        size: args.size ?? "default",
        value: args.value ?? args.defaultValue ?? "",
        controlled: args.value !== undefined,
        readOnly: Boolean(args.readOnly),
        placeholder: args.placeholder ?? "",
        ariaLabel: args["aria-label"] ?? "",
        invalid: Boolean(args["aria-invalid"]),
        icon: name === "With Icon" ? "plus" : "",
        ...(title === "Components/Button" && name === "Default"
          ? {
              interactions: [
                { action: "click", role: "button", name: text(args.children), exact: true },
                { action: "callback", name: "onClick", count: 1 },
              ],
            }
          : {}),
        ...(title === "Components/Input" && name === "Keyboard Input"
          ? {
              interactions: [
                { action: "type", role: "textbox", value: "design-system" },
                { action: "wait", role: "textbox", value: "design-system" },
              ],
            }
          : {}),
      },
    };
  }
  if (title === "Components/Dialog" || title === "Components/AlertDialog") {
    const alert = title === "Components/AlertDialog";
    return {
      ...base,
      sample: "dialog-demo",
      fixture: {
        ...fixture,
        ...dialogSamples[alert ? "alert" : "dialog"],
        alert,
        // React's custom render mounts DialogDemo with defaultOpen for this state.
        open: name === "Open",
        interactions:
          name === "Open" ? [{ action: "wait", role: alert ? "alertdialog" : "dialog" }] : [],
      },
    };
  }
  if (title === "Components/ConfirmActionDialog")
    return {
      ...base,
      sample: "confirm",
      fullHeight: true,
      fixture: {
        ...fixture,
        title: args.title,
        description: args.description,
        confirm: args.confirmLabel,
        open: args.open,
        pending: args.pending,
        error: args.error,
        interactions:
          name === "Default"
            ? [
                { action: "click", role: "button", name: args.confirmLabel, exact: true },
                { action: "callback", name: "onConfirm", count: 1 },
                { action: "click", role: "button", name: "キャンセル", exact: true },
                { action: "callback", name: "onOpenChange", args: [false] },
              ]
            : [],
      },
    };
  if (title === "Components/ControlPanel") {
    const row = record(record(args.children).props);
    const action = record(record(row.children).props);
    return {
      ...base,
      sample: "control-panel",
      fixture: {
        ...fixture,
        title: args.title,
        description: args.description,
        rowLabel: row.label,
        rowDescription: row.description,
        actionLabel: action.children,
      },
    };
  }
  if (title === "Components/Select") {
    function options(node: unknown): { value: string; label: string }[] {
      if (Array.isArray(node)) return node.flatMap(options);
      const element = record(node),
        props = record(element.props);
      if (element.type === "option")
        return [
          {
            value: typeof props.value === "string" ? props.value : "",
            label: typeof props.children === "string" ? props.children : "",
          },
        ];
      return props.children ? options(props.children) : [];
    }
    const choices = options(args.children);
    const value = args.value ?? args.defaultValue;
    return {
      ...base,
      sample: "select",
      fixture: {
        ...fixture,
        ariaLabel: args["aria-label"],
        options: choices.map((choice) => choice.label),
        optionValues: choices.map((choice) => choice.value),
        controlled: args.value !== undefined,
        value: choices.find((choice) => choice.value === value)?.label ?? choices[0]?.label ?? "",
      },
    };
  }
  if (title === "Components/Avatar" || title === "Components/Skeleton")
    return {
      ...base,
      sample: title === "Components/Avatar" ? "avatar" : "skeleton",
      intrinsicWidth: title === "Components/Skeleton" ? (name === "Avatar" ? 40 : 256) : undefined,
      fixture: {
        ...fixture,
        avatarInitials: avatarStoryInitials,
        state: name,
        size: args.size ?? "default",
        className: args.className ?? "",
      },
    };
  if (
    title === "Features/Chat/MessageRow" &&
    [
      "Default",
      "Own Message",
      "Grouped",
      "With Reactions",
      "Reaction Users",
      "Reaction Users Hover",
      "Long Message",
      "Card",
      "With Attachments",
      "App Card Block",
    ].includes(name)
  ) {
    const item = record(args.item);
    const message: Record<string, unknown> = {
      ...record(item.message),
      ...(args.isOwn ? { user_id: fixture.currentUserId } : {}),
    };
    const groups = Array.isArray(args.reactions) ? args.reactions.map(record) : [];
    const users = new Map(
      (fixture.users as Record<string, unknown>[]).map((user) => [user.id, user]),
    );
    users.set(message.user_id, {
      id: message.user_id,
      user_name: message.user_name,
      display_name: message.display_name,
    });
    const reactions = groups.flatMap((group, index) => {
      const names = record(group.userNames);
      return (Array.isArray(group.userIds) ? group.userIds : []).map((userId) => {
        const label = names[String(userId)];
        if (typeof label === "string") {
          const match = /^(.*?) \(@(.*?)\)$/.exec(label);
          users.set(userId, {
            id: userId,
            display_name: match?.[1] ?? "",
            user_name: match?.[2] ?? label.replace(/^@/, ""),
          });
        }
        return {
          id: `reaction-${index}-${userId}`,
          message_id: message.id,
          user_id: userId,
          emoji: group.emoji,
        };
      });
    });
    return {
      ...base,
      content: typeof message.content === "string" ? message.content : "",
      grouped: !item.isGroupStart,
      fixture: {
        ...fixture,
        ...message,
        attachments: args.attachments ?? [],
        users: [...users.values()],
        reactions,
        replyCount: args.replyCount ?? 0,
      },
    };
  }
  if (title === "Features/Chat/MessageContent")
    return {
      ...base,
      sample: "rich-text",
      content: typeof args.content === "string" ? args.content : "",
    };
  if (title === "Features/Chat/MessageCard" && name !== "Git Hub Feed")
    return {
      ...base,
      sample: "card",
      fixture: { ...fixture, card: args.card, attachment: Boolean(args.attachment) },
    };
  if (title === "Features/Chat/LinkPreviews")
    return {
      ...base,
      sample: "link-preview",
      content: typeof args.content === "string" ? args.content : "",
    };
  if (title === "Features/Chat/Composer" && ["Default", "Disabled", "Send Error"].includes(name))
    return {
      ...base,
      sample: "composer",
      fixture: { ...fixture, postFailure: name === "Send Error" },
    };
  if (title === "Features/Chat/ReactionPicker" && name === "Default")
    return { ...base, sample: "emoji-picker" };
  if (title === "Features/Apps/Blocks" && name === "Card")
    return {
      ...base,
      sample: "app-blocks",
      fixture: { ...fixture, blocks: args.blocks, appId: args.appId, submit: args.submit },
    };
  if (title === "Pages/ZooSettings" && ["General", "General Mobile", "Members"].includes(name))
    return { ...base, sample: name === "Members" ? "settings-members" : "settings" };
  if (title === "Pages/BubbleChat" && name === "Empty") return { ...base, sample: "bubble" };
  if (
    title === "Pages/Notifications" &&
    ["Default", "Loading", "Empty", "Mobile", "Dark", "Single Unread", "Direct Messages"].includes(
      name,
    )
  )
    return { ...base, sample: name === "Loading" ? "notifications-loading" : "notifications" };
  return null;
}
