import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, screen, userEvent, waitFor, within } from "storybook/test";
import { THEME_STORAGE_KEY } from "./theme";
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

export const Default: Story = {
  play: async () => {
    const group = within(screen.getByRole("group", { name: "テーマ" }));
    await expect(group.getAllByRole("button")).toHaveLength(3);
    await expect(
      group
        .getAllByRole("button")
        .filter((button) => button.getAttribute("aria-pressed") === "true"),
    ).toHaveLength(1);
    await expect(group.getAllByRole("button").every((button) => button.textContent === "")).toBe(
      true,
    );
  },
};
export const Dark: Story = {
  play: async () => {
    const dark = screen.getByRole("button", { name: "ダーク" });
    await userEvent.click(dark);
    await expect(dark).toHaveAttribute("aria-pressed", "true");
    await expect(document.documentElement).toHaveClass("dark");
    await expect(localStorage.getItem(THEME_STORAGE_KEY)).toBe("dark");
    await userEvent.click(dark);
    await expect(dark).toHaveAttribute("aria-pressed", "true");
  },
};
export const KeyboardSelection: Story = {
  globals: { theme: "light" },
  play: async () => {
    await userEvent.tab();
    await expect(screen.getByRole("button", { name: "ライト" })).toHaveFocus();
    await userEvent.keyboard("{ArrowRight}{ArrowRight}{Enter}");
    await expect(screen.getByRole("button", { name: "システム" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    await expect(localStorage.getItem(THEME_STORAGE_KEY)).toBe("system");
    await userEvent.keyboard("{ArrowLeft} ");
    await expect(screen.getByRole("button", { name: "ダーク" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    await expect(document.documentElement).toHaveClass("dark");
  },
};
export const Explanation: Story = {
  play: async () => {
    await userEvent.hover(screen.getByRole("button", { name: "システム" }));
    const tooltip = await screen.findByText("システム");
    await waitFor(() => expect(tooltip).toBeVisible());
  },
};
