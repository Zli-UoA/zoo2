import type { StoryContext } from "@storybook/react-vite";
import type { KotlinPreviewProps } from "./KotlinPreview";
import { isLocale } from "../src/lib/i18n/locale";
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
    sharedApi: true,
    currentUserId: collections.currentUserId ?? storyCollections.currentUserId,
    isAdmin: title === "Pages/ZooSettings" && name === "Members",
    users: collections.users ?? storyCollections.users,
    channels: collections.channels ?? storyCollections.channels,
    notifications: collections.notifications ?? storyCollections.notifications,
  };
  const base: KotlinPreviewProps = {
    sample: "message",
    content: "",
    disabled: Boolean(args.disabled),
    grouped: false,
    locale: isLocale(globals.locale) ? globals.locale : "ja",
    dark: globals.theme === "dark",
    fixture,
  };
  if (
    title === "Features/Chat/MessageRow" &&
    [
      "Default",
      "Own Message",
      "Grouped",
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
    return {
      ...base,
      content: typeof message.content === "string" ? message.content : "",
      grouped: !item.isGroupStart,
      fixture: {
        ...fixture,
        ...message,
        attachments: args.attachments ?? [],
        users: [
          { id: message.user_id, user_name: message.user_name, display_name: message.display_name },
        ],
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
