import type { Meta, StoryObj } from "@storybook/react-vite";

import { ShellSkeleton } from "./ShellSkeleton";

const meta = {
  title: "Pages/Skeleton",
  component: ShellSkeleton,
  tags: ["autodocs"],
  parameters: { a11y: { test: "error" } },
} satisfies Meta<typeof ShellSkeleton>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};

export const Mobile: Story = {
  globals: { viewport: { value: "mobile" } },
};
