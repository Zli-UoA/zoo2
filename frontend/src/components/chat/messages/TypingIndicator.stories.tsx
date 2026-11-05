import type { Meta, StoryObj } from "@storybook/react-vite";

import { TypingIndicator } from "./TypingIndicator";

const meta = {
  title: "Features/Chat/Typing",
  component: TypingIndicator,
  tags: ["autodocs"],
  parameters: { a11y: { test: "error" } },
  args: {
    users: [],
  },
  decorators: [
    (Story) => (
      <div className="bg-background py-4">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof TypingIndicator>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Empty: Story = {};

export const Single: Story = {
  args: {
    users: [{ id: "user-bob", display_name: "Bob" }],
  },
};

export const Multiple: Story = {
  args: {
    users: [
      { id: "user-bob", display_name: "Bob" },
      { id: "user-carol", display_name: "Carol" },
      { id: "user-dave", display_name: "Dave" },
    ],
  },
};
