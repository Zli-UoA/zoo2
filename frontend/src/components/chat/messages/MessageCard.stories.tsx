import type { Meta, StoryObj } from "@storybook/react-vite";

import { MessageCard, type CardData } from "./MessageCard";

const card: CardData = {
  title: "Deploy complete",
  description: "production へのデプロイが完了しました。",
  url: "https://example.com/deployments/42",
  source: "Deployment service",
  context: "zoo2 / production",
  author: "Deployment bot",
  badge: "success",
};

const meta = {
  title: "Chat/MessageCard",
  component: MessageCard,
  tags: ["autodocs"],
  parameters: { a11y: { test: "error" } },
  args: {
    card,
  },
  decorators: [
    (Story) => (
      <div className="mx-auto max-w-3xl bg-background p-6">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof MessageCard>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};

export const TitleOnly: Story = {
  args: {
    card: {
      ...card,
      description: "",
      source: "",
      context: "",
      author: "",
      badge: "",
    },
  },
};

export const LongDescription: Story = {
  args: {
    card: {
      ...card,
      title: "長いデプロイ結果のサマリー",
      description: Array.from(
        { length: 6 },
        (_, index) =>
          `${index + 1}. チェック項目の確認結果と、次回のデプロイに向けた注意点をまとめています。`,
      ).join("\n"),
      badge: "review needed",
    },
  },
};
