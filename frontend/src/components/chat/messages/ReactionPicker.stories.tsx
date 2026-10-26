import type { Meta, StoryObj } from "@storybook/react-vite";
import { fn } from "storybook/test";
import { useState } from "react";

import { Button } from "../../ui/button";
import { ReactionPicker } from "./ReactionPicker";

function ReactionPickerDemo({ onSelect }: { onSelect: (emoji: string) => void }) {
  const [open, setOpen] = useState(true);
  return (
    <div className="flex min-h-96 items-center justify-center bg-background p-8">
      <Button onClick={() => setOpen(true)}>リアクションを追加</Button>
      <ReactionPicker open={open} onOpenChange={setOpen} onSelect={onSelect} />
    </div>
  );
}

const meta = {
  title: "Chat/ReactionPicker",
  component: ReactionPicker,
  args: {
    open: true,
    onOpenChange: fn(),
    onSelect: fn(),
  },
  render: (args) => <ReactionPickerDemo onSelect={args.onSelect} />,
} satisfies Meta<typeof ReactionPicker>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};
