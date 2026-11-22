import type { Meta, StoryObj } from "@storybook/react-vite";
import { http, HttpResponse } from "msw";

import { CatchUpButton } from "./CatchUpButton";

const states = [
  {
    channel_id: "one",
    last_read_at: null,
    has_unread: true,
    unread_count: 3,
    unread_count_truncated: false,
    first_unread_at: "2026-01-01T00:00:00Z",
  },
  {
    channel_id: "two",
    last_read_at: null,
    has_unread: true,
    unread_count: 1,
    unread_count_truncated: false,
    first_unread_at: "2026-01-02T00:00:00Z",
  },
];

const meta = {
  title: "Features/CatchUp/Button",
  component: CatchUpButton,
  tags: ["autodocs"],
  parameters: { a11y: { test: "error" } },
  decorators: [
    (Story) => (
      <div className="flex items-center bg-background p-6">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof CatchUpButton>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Unread: Story = {
  parameters: {
    msw: { handlers: [http.get("*/api/channel-read-states", () => HttpResponse.json(states))] },
  },
};

export const AllRead: Story = {
  parameters: {
    msw: { handlers: [http.get("*/api/channel-read-states", () => HttpResponse.json([]))] },
  },
};
