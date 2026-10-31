import type { Meta, StoryObj } from "@storybook/react-vite";

import { StoryShell } from "../../../.storybook/shell";
import { LeftRail } from "./LeftRail";

const meta = {
  title: "Layout/LeftRail",
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

export const Default: Story = {};

export const Mobile: Story = {
  globals: { viewport: { value: "mobile" } },
};
