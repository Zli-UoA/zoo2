import { storyDeploymentCard } from "../../../../.storybook/app-fixtures";
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
  title: "Features/Chat/MessageCard",
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

export const Deployment: Story = { args: { card: storyDeploymentCard, attachment: true } };
export const GitHubFeed: Story = {
  args: { card: storyDeploymentCard },
  render: () => (
    <div className="max-w-2xl space-y-7 p-4">
      <section>
        <p className="text-sm">Deployment to storybook-pr-271 by Kai-Z-JP</p>
        <MessageCard card={storyDeploymentCard} attachment />
      </section>
      <section>
        <p className="text-sm">1 new commit pushed to main by Kai-Z-JP</p>
        <MessageCard
          attachment
          card={{
            ...storyDeploymentCard,
            title: "Zli-UoA/zoo2 · 103eb20",
            color: "#24292f",
            fields: [
              {
                title: "103eb20",
                value: "fix: preserve queued webhooks while Zoo credentials are revoked (#271)",
                url: "https://github.com/Zli-UoA/zoo2/commit/103eb20",
              },
            ],
          }}
        />
      </section>
      <section>
        <p className="text-sm">Pull request merged by Kai-Z-JP</p>
        <MessageCard
          attachment
          card={{
            ...storyDeploymentCard,
            title: "#271 fix: Zoo 資格情報の失効中も Bot の受信キューを保持する",
            color: "#8250df",
            url: "https://github.com/Zli-UoA/zoo2/pull/271",
            fields: [],
          }}
        />
      </section>
    </div>
  ),
};
