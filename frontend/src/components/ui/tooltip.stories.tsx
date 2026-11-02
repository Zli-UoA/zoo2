import { tooltipExample } from "../../../.storybook/primitive-fixtures";
import { playForImplementation } from "../../../.storybook/play-for-implementation";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, within, userEvent } from "storybook/test";

import { Button } from "./button";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "./tooltip";

function TooltipDemo() {
  return (
    <div className="flex min-h-72 items-center justify-center bg-background p-12">
      <TooltipProvider>
        <Tooltip>
          <TooltipTrigger render={<Button variant="outline" />}>
            {tooltipExample.trigger}
          </TooltipTrigger>
          <TooltipContent>{tooltipExample.content}</TooltipContent>
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
  play: playForImplementation(async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const portal = within(canvasElement.ownerDocument.body);
    await userEvent.hover(await canvas.findByRole("button", { name: "メッセージを編集" }));
    await expect(await portal.findByText("メッセージを編集できます")).toBeInTheDocument();
  }),
};
