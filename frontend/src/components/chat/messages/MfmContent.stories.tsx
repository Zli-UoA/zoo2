import type { Meta, StoryObj } from "@storybook/react-vite";
import { MfmChannelsProvider } from "./MfmChannels";
import { storyChannels } from "../../../../.storybook/fixtures";
import { MfmContent } from "./MfmContent";
const meta = {
  title: "Features/Chat/MessageContent",
  component: MfmContent,
  parameters: { a11y: { test: "error" } },
  decorators: [
    (Story) => (
      <div className="mx-auto max-w-2xl whitespace-pre-wrap break-words bg-background p-4 text-sm text-foreground">
        <MfmChannelsProvider channels={storyChannels}>
          <Story />
        </MfmChannelsProvider>
      </div>
    ),
  ],
  args: {
    emojiUrls: {},
    content:
      "こんにちは @alice **リリース情報**を共有します。\n[ドキュメント](https://example.com/docs)を確認してください。 #general",
  },
} satisfies Meta<typeof MfmContent>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Default: Story = {};
export const InlineLinks: Story = {
  args: {
    content:
      "本文の途中にhttps://example.com/docsがあります。\n[表示名のあるリンク](https://example.com)と前後の余白を確認します。",
  },
};
export const LongUrl: Story = {
  args: { content: "URL: https://example.com/" + "long-path-segment/".repeat(24) },
  globals: { viewport: { value: "mobile" } },
};
export const CodeAndMath: Story = {
  args: {
    content:
      "`const zoo = true`\n```ts\nconst channel = { name: 'general' };\n```\n数式: \\(x^2 + y^2\\)",
  },
};
export const EmojiAndFallback: Story = {
  args: { content: "😀 🎉 :smile: :unregistered_zoo_emoji:" },
};
export const Formatting: Story = {
  args: { content: "**強調** *斜体* ~~取り消し~~\n> 引用文\n$[ruby 漢字 かんじ]" },
};

export const CodeAndMathDark: Story = { ...CodeAndMath, globals: { theme: "dark" } };
