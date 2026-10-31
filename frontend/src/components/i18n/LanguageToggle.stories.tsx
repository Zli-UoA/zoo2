import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, screen, userEvent } from "storybook/test";

import { LanguageToggle } from "./LanguageToggle";

const meta = {
  title: "Features/Preferences/Language",
  component: LanguageToggle,
  tags: ["autodocs"],
  parameters: { a11y: { test: "error" } },
  decorators: [
    (Story) => (
      <div className="flex min-h-72 items-center justify-center bg-background p-6">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof LanguageToggle>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};

export const Open: Story = {
  play: async () => {
    await userEvent.click(screen.getByRole("button", { name: "言語を切り替える" }));
    await expect(await screen.findByRole("menuitemradio", { name: "日本語" })).toBeInTheDocument();
  },
};
