import type { Meta, StoryObj } from "@storybook/react-vite";
import { fn } from "storybook/test";

import { UnbackedKeysLogoutDialog } from "./UnbackedKeysLogoutDialog";

const meta = {
  title: "Features/E2EE/UnbackedKeysLogoutDialog",
  component: UnbackedKeysLogoutDialog,
  parameters: { a11y: { test: "error" } },
  args: { count: 3, onCancel: fn(), onConfirm: fn() },
} satisfies Meta<typeof UnbackedKeysLogoutDialog>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};

export const Pending: Story = { args: { pending: true } };
