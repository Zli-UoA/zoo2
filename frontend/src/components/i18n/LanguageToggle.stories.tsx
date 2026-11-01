import { preferenceSurfaces } from "../../../.storybook/preference-stories";
import { playForImplementation } from "../../../.storybook/play-for-implementation";
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
      <div className={preferenceSurfaces["Features/Preferences/Language"]}>
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof LanguageToggle>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};

export const Open: Story = {
  play: playForImplementation(async () => {
    await userEvent.click(await screen.findByRole("button", { name: "言語を切り替える" }));
    await expect(await screen.findByRole("menuitemradio", { name: "日本語" })).toBeInTheDocument();
  }),
};

export const SelectControl: Story = {
  args: { presentation: "select" },
  play: playForImplementation(async () => {
    const select = await screen.findByRole("combobox", { name: "言語" });
    await expect(screen.getAllByRole("option")).toHaveLength(5);
    await userEvent.selectOptions(select, "en");
    await expect(await screen.findByRole("combobox", { name: "Language" })).toHaveValue("en");
    await userEvent.selectOptions(select, "ja");
    await expect(await screen.findByRole("combobox", { name: "言語" })).toHaveValue("ja");
  }),
};
