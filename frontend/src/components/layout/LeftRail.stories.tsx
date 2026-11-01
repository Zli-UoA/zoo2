import { playForImplementation } from "../../../.storybook/play-for-implementation";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, within } from "storybook/test";

import { StoryShell } from "../../../.storybook/shell";
import { LeftRail } from "./LeftRail";

const meta = {
  title: "Features/Navigation/Rail",
  component: LeftRail,
  tags: ["autodocs"],
  parameters: { a11y: { test: "error" } },
  decorators: [
    (Story) => (
      <StoryShell>
        <div className="flex h-[640px] bg-background">
          <Story />
        </div>
      </StoryShell>
    ),
  ],
} satisfies Meta<typeof LeftRail>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {
  play: playForImplementation(async ({ canvasElement }) => {
    await expect(
      await within(canvasElement).findByRole("button", {
        name: "Compose版を使う（実験・開発中）",
      }),
    ).toBeVisible();
  }),
};

export const Mobile: Story = {
  ...Default,
  globals: { viewport: { value: "mobile" } },
};
