import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, screen, userEvent } from "storybook/test";

import { Button } from "./button";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "./tooltip";

function TooltipDemo() {
  return (
    <div className="flex min-h-72 items-center justify-center bg-background p-12">
      <TooltipProvider>
        <Tooltip>
          <TooltipTrigger render={<Button variant="outline" />}>メッセージを編集</TooltipTrigger>
          <TooltipContent>メッセージを編集できます</TooltipContent>
        </Tooltip>
      </TooltipProvider>
    </div>
  );
}

const meta = {
  title: "Components/Tooltip",
  component: Tooltip,
  tags: ["autodocs"],
  parameters: { a11y: { test: "error" } },
  render: () => <TooltipDemo />,
} satisfies Meta<typeof Tooltip>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};

export const Open: Story = {
  play: async () => {
    await userEvent.hover(screen.getByRole("button", { name: "メッセージを編集" }));
    await expect(await screen.findByText("メッセージを編集できます")).toBeInTheDocument();
  },
};
