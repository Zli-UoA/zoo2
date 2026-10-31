import type { Meta, StoryObj } from "@storybook/react-vite";
import { isLocale } from "../../lib/i18n/locale";
import { storyDeploymentCard } from "../../../.storybook/app-fixtures";
import { ComposeCatalog } from "./ComposeCatalog";

const meta = {
  title: "Features/Compose/Chat",
  component: ComposeCatalog,
  // Canvas interactions are covered by Playwright against the built catalog.
  // Existing React story tests can run without a JDK or generated Wasm assets.
  tags: ["!test"],
  parameters: {
    layout: "fullscreen",
    // axe in the Storybook parent cannot inspect Canvas semantics in this iframe.
    // The real Kotlin controls are exercised by e2e/compose-storybook.spec.ts;
    // this exception does not claim screen-reader/a11y coverage of the Canvas UI.
    a11y: { test: "off" },
  },
  args: {
    compare: false,
    sample: "message",
    content: "",
    disabled: false,
    grouped: false,
    dark: false,
    locale: "ja",
  },
  argTypes: {
    compare: { control: false, table: { disable: true } },
    sample: {
      control: "select",
      options: [
        "message",
        "composer",
        "emoji-picker",
        "rich-text",
        "link-preview",
        "settings",
        "settings-members",
        "bubble",
        "notifications",
        "notifications-loading",
      ],
    },
    locale: { control: false, table: { disable: true } },
    dark: { control: false, table: { disable: true } },
  },
  render: (args, context) => (
    <ComposeCatalog
      {...args}
      dark={context.globals.theme === "dark"}
      locale={isLocale(context.globals.locale) ? context.globals.locale : "ja"}
    />
  ),
} satisfies Meta<typeof ComposeCatalog>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Message: Story = {};
export const GroupedMessage: Story = { args: { grouped: true } };
export const Composer: Story = { args: { sample: "composer" } };
export const DisabledComposer: Story = { args: { sample: "composer", disabled: true } };
export const EmojiPicker: Story = { args: { sample: "emoji-picker" } };
export const RichText: Story = { args: { sample: "rich-text" } };

export const ReactComparison: Story = {
  args: { compare: true },
  argTypes: { sample: { control: false } },
};

export const LongMessage: Story = {
  args: {
    content:
      "長い日本語の本文とURLが隣の投稿や操作を覆わないことを確認します。".repeat(4) +
      "\nhttps://example.com/" +
      "long-path/".repeat(24),
  },
};

export const LinkPreview: Story = {
  args: { sample: "link-preview", content: "https://example.com/article" },
};
export const IntegrationCards: Story = {
  args: { fixture: { card: JSON.stringify({ cards: [storyDeploymentCard] }) } },
};
export const AppCardBlock: Story = {
  args: {
    content: "この本文はカードと二重表示しません",
    fixture: {
      app_id: "catalog-app",
      app_blocks: JSON.stringify([{ type: "card", card: storyDeploymentCard }]),
    },
  },
};
export const Settings: Story = { args: { sample: "settings" } };
export const WorkspaceMembers: Story = { args: { sample: "settings-members" } };
export const BubbleChat: Story = { args: { sample: "bubble" } };
export const Notifications: Story = { args: { sample: "notifications" } };
export const NotificationsLoading: Story = { args: { sample: "notifications-loading" } };
