import type { Meta, StoryObj } from "@storybook/react-vite";

import { StoryShell } from "../../../.storybook/shell";
import { storyCollections } from "../../../.storybook/fixtures";
import { NotificationsScreen } from "./NotificationsScreen";

const meta = {
  title: "Notifications/NotificationsScreen",
  component: NotificationsScreen,
  tags: ["autodocs"],
  parameters: { a11y: { test: "error" } },
  decorators: [
    (Story) => (
      <StoryShell selectedChannelId={null} initialPane="chat">
        <div className="h-[720px] overflow-hidden bg-background">
          <Story />
        </div>
      </StoryShell>
    ),
  ],
} satisfies Meta<typeof NotificationsScreen>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};

export const Empty: Story = {
  parameters: {
    collections: { ...storyCollections, notifications: [] },
  },
};

export const Mobile: Story = {
  globals: { viewport: { value: "mobile" } },
};
