import { storySurfaces } from "../../../../.storybook/story-surfaces";
import { playForImplementation } from "../../../../.storybook/play-for-implementation";
import { expect, within } from "storybook/test";
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
      <div className={storySurfaces["Features/Chat/MessageContent"]}>
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

export const Headings: Story = {
  args: {
    content:
      "# リリースのお知らせ\n通常の本文です。\n## **変更点**と[ドキュメント](https://example.com/docs)\n### チャンネル #general\n#### 小さな見出し\n##### 補足\n###### 詳細\n\n```md\n# コード内はそのまま\n```\n<plain>## 装飾しない</plain>",
  },
  play: playForImplementation(async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    for (let level = 1; level <= 6; level++) {
      await expect(await canvas.findByRole("heading", { level })).toBeVisible();
    }
    await expect(canvas.getByRole("link", { name: "ドキュメント" })).toHaveAttribute(
      "href",
      "https://example.com/docs",
    );
    await expect(canvas.getByText("# コード内はそのまま")).toBeVisible();
    await expect(canvasElement).toHaveTextContent("## 装飾しない");
    await expect(canvas.getAllByRole("heading")).toHaveLength(6);
  }),
};
