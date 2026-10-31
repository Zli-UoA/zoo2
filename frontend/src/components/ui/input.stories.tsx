import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, userEvent, within } from "storybook/test";
import { Input } from "./input";
const meta = {
  title: "Components/Input",
  component: Input,
  parameters: { layout: "centered", a11y: { test: "error" } },
  args: { "aria-label": "チャンネル名", placeholder: "general" },
} satisfies Meta<typeof Input>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Default: Story = {};
export const Disabled: Story = { args: { disabled: true, value: "general" } };
export const Invalid: Story = { args: { "aria-invalid": true, defaultValue: "使えない名前" } };
export const LongValue: Story = { args: { defaultValue: "very-long-channel-name-".repeat(8) } };
export const KeyboardInput: Story = {
  play: async ({ canvasElement }) => {
    const input = within(canvasElement).getByRole("textbox");
    await userEvent.type(input, "design-system");
    await expect(input).toHaveValue("design-system");
    await expect(input).toHaveAttribute("data-1p-ignore");
  },
};
