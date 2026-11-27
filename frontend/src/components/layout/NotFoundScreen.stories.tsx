import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, screen } from "storybook/test";

import { NotFoundScreen } from "./NotFoundScreen";

const meta = {
  title: "Pages/Shell/NotFound",
  component: NotFoundScreen,
  parameters: { a11y: { test: "error" } },
  decorators: [
    (Story) => (
      <div className="h-dvh">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof NotFoundScreen>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {
  play: async () => {
    await expect(
      await screen.findByRole("heading", { level: 1, name: "ページが見つかりません" }),
    ).toBeVisible();
    await expect(screen.getByRole("link", { name: "チャンネル一覧へ戻る" })).toHaveAttribute(
      "href",
      "/",
    );
  },
};

export const Mobile: Story = {
  globals: { viewport: { value: "mobile" } },
};
