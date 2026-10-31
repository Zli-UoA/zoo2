import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, fn, screen, userEvent } from "storybook/test";
import { StoryShell } from "../../../.storybook/shell";
import { storyCollections } from "../../../.storybook/fixtures";
import { CreateDmDialog } from "./CreateDmDialog";
const meta = {
  title: "Features/DMs/Create",
  component: CreateDmDialog,
  parameters: { a11y: { test: "error" } },
  decorators: [
    (Story) => (
      <StoryShell>
        <Story />
      </StoryShell>
    ),
  ],
  args: { open: true, onOpenChange: fn() },
} satisfies Meta<typeof CreateDmDialog>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Default: Story = {};
export const Selected: Story = {
  play: async () => {
    const search = await screen.findByRole("combobox");
    await userEvent.type(search, "Bob");
    await userEvent.click(await screen.findByRole("option", { name: /Bob/ }));
    await expect(screen.getByRole("button", { name: /Bob.*削除|Bob.*外す/ })).toBeVisible();
  },
};
export const SearchEmpty: Story = {
  play: async () => {
    await userEvent.type(await screen.findByRole("combobox"), "no-such-user");
    await expect(screen.queryByRole("option")).not.toBeInTheDocument();
  },
};
export const Empty: Story = { parameters: { collections: { ...storyCollections, users: [] } } };
export const Mobile: Story = { globals: { viewport: { value: "mobile" } } };
