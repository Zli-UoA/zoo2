import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, screen, userEvent } from "storybook/test";

import { ModeToggle } from "./ModeToggle";

const meta = {
  title: "Features/Preferences/Theme",
  component: ModeToggle,
  tags: ["autodocs"],
  parameters: { a11y: { test: "error" } },
  decorators: [
    (Story) => (
      <div className="flex min-h-72 items-center justify-center bg-background p-6">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof ModeToggle>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};

export const MenuOpen: Story = {
  play: async () => {
    await userEvent.click(screen.getByRole("button", { name: "テーマを切り替える" }));
    await expect(await screen.findByRole("menuitemradio", { name: "ダーク" })).toBeInTheDocument();
  },
};

export const SelectControl: Story = {
  args: { presentation: "select" },
  play: async () => {
    const select = screen.getByRole("combobox", { name: "テーマ" });
    await userEvent.selectOptions(select, "dark");
    await expect(select).toHaveValue("dark");
    await expect(document.documentElement).toHaveClass("dark");
    await userEvent.selectOptions(select, "system");
    await expect(select).toHaveValue("system");
  },
};
