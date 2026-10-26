import type { Meta, StoryObj } from "@storybook/react-vite";
import { fn } from "storybook/test";

import { Composer } from "./Composer";

const meta = {
  title: "Chat/Composer",
  component: Composer,
  args: {
    onTyping: fn(),
    onSend: async () => true,
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
