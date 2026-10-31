import { playForImplementation } from "../../../.storybook/play-for-implementation";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { HashIcon, PlusIcon } from "lucide-react";
import { expect, fn, userEvent, within } from "storybook/test";
import { CollapsibleSection } from "./collapsible-section";
import { Button } from "./button";
const meta = {
  title: "Components/CollapsibleSection",
  component: CollapsibleSection,
  parameters: { a11y: { test: "error" } },
  decorators: [
    (Story) => (
      <div className="w-72 bg-sidebar">
        <Story />
      </div>
    ),
  ],
  args: {
    title: "チャンネル",
    icon: <HashIcon />,
    count: 2,
    children: <div className="px-4 py-2 text-sm">general / design</div>,
  },
} satisfies Meta<typeof CollapsibleSection>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Default: Story = {};
export const Closed: Story = { args: { defaultOpen: false } };
export const WithActions: Story = {
  args: {
    actions: (
      <Button variant="ghost" size="icon-sm" aria-label="チャンネルを作成" onClick={fn()}>
        <PlusIcon />
      </Button>
    ),
  },
  play: playForImplementation(async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const toggle = canvas.getByRole("button", { name: "チャンネル" });
    await userEvent.click(canvas.getByRole("button", { name: "チャンネルを作成" }));
    await expect(toggle).toHaveAttribute("aria-expanded", "true");
    await userEvent.click(toggle);
    await expect(canvas.getByText("general / design")).not.toBeVisible();
    await userEvent.keyboard("{Enter}");
    await expect(canvas.getByText("general / design")).toBeVisible();
    await userEvent.keyboard(" ");
    await expect(toggle).toHaveAttribute("aria-expanded", "false");
  }),
};
