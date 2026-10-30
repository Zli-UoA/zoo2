import type { Meta, StoryObj } from "@storybook/react-vite";

import { WelcomePanel } from "./WelcomePanel";

const meta = {
  title: "Layout/WelcomePanel",
  component: WelcomePanel,
  tags: ["autodocs"],
  parameters: { a11y: { test: "error" } },
  decorators: [
    (Story) => (
      <div className="h-[640px] bg-background">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof WelcomePanel>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};

export const Mobile: Story = {
  globals: { viewport: { value: "mobile" } },
};
