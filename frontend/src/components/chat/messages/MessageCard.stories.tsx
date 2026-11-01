import { githubFeedCards } from "../../../../.storybook/message-fixtures";
import { storySurfaces } from "../../../../.storybook/story-surfaces";
import { playForImplementation } from "../../../../.storybook/play-for-implementation";
import { expect, userEvent, within } from "storybook/test";
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
      <div className={storySurfaces["Features/Chat/MessageCard"]}>
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
      {githubFeedCards.map(({ body, card }) => (
        <section key={body}>
          <p className="text-sm">{body}</p>
          <MessageCard card={card} attachment />
        </section>
      ))}
    </div>
  ),
};

export const CollapsibleBody: Story = {
  args: {
    attachment: true,
    card: {
      ...storyDeploymentCard,
      description: Array.from(
        { length: 20 },
        (_, i) => `本文 ${i + 1}: リリース内容の詳細です。`,
      ).join("\n"),
      author: "Deployment bot",
    },
  },
  play: playForImplementation(async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const toggle = await canvas.findByRole("button", { name: "もっと見る" });
    await expect(toggle).toHaveAttribute("aria-expanded", "false");
    await expect(canvas.getByText("Completed")).toBeVisible();
    await expect(canvas.getByText("Deployment bot")).toBeVisible();
    await userEvent.click(toggle);
    await expect(canvas.getByRole("button", { name: "折りたたむ" })).toHaveAttribute(
      "aria-expanded",
      "true",
    );
    await userEvent.click(toggle);
    await expect(toggle).toHaveAttribute("aria-expanded", "false");
  }),
};
