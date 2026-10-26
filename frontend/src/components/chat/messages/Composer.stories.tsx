import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, fn, userEvent, within } from "storybook/test";

import type { UserRow } from "../../../lib/electric/types";
import { Composer } from "./Composer";

const mentionUsers: UserRow[] = [
  {
    id: "user-alice",
    user_name: "alice",
    display_name: "Alice",
    created_at: "2026-01-01 00:00:00.000",
    updated_at: null,
  },
  {
    id: "user-bob",
    user_name: "bob",
    display_name: "Bob",
    created_at: "2026-01-01 00:00:00.000",
    updated_at: null,
  },
];

const meta = {
  title: "Chat/Composer",
  component: Composer,
  tags: ["autodocs"],
  parameters: { a11y: { test: "todo" } },
  argTypes: {
    disabled: { control: "boolean" },
    placeholder: { control: "text" },
    ariaLabel: { control: "text" },
  },
  args: {
    onTyping: fn(),
    onSend: async () => true,
    mentionUsers,
    mentionMemberIds: new Set(["user-alice", "user-bob"]),
  },
  decorators: [
    (Story) => (
      <div className="mx-auto max-w-3xl bg-background">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof Composer>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};

export const Disabled: Story = {
  args: {
    disabled: true,
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByRole("combobox")).toBeDisabled();
  },
};

export const Multiline: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const textarea = canvas.getByRole("combobox");
    await userEvent.type(textarea, "1行目{enter}2行目{enter}3行目");
    await expect(textarea).toHaveValue("1行目\n2行目\n3行目");
  },
};

export const MentionSuggestions: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.type(canvas.getByRole("combobox"), "@al");
    await expect(canvas.getByRole("listbox", { name: "メンション候補" })).toBeVisible();
  },
};

export const Mobile: Story = {
  globals: { viewport: { value: "mobile" } },
  args: {
    placeholder: "モバイルからメッセージを送信",
  },
};
