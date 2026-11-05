import type { Meta, StoryObj } from "@storybook/react-vite";
import { Skeleton } from "./skeleton";
const meta = {
  title: "Components/Skeleton",
  component: Skeleton,
  parameters: { layout: "centered", a11y: { test: "error" } },
  args: { className: "h-4 w-64" },
} satisfies Meta<typeof Skeleton>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Text: Story = {};
export const Avatar: Story = { args: { className: "size-10 rounded-full" } };
