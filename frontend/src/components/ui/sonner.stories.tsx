import { toastExample } from "../../../.storybook/primitive-fixtures";
import { playForImplementation } from "../../../.storybook/play-for-implementation";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { toast } from "sonner";
import { expect, within, userEvent, waitFor } from "storybook/test";
import { Toaster } from "./sonner";
import { Button } from "./button";
const meta = {
  title: "Components/Toaster",
  component: Toaster,
  parameters: { a11y: { test: "error" } },
  render: () => (
    <div className="p-4">
      <Button
        onClick={() =>
          toast.success(toastExample.content, { id: "storybook-toast", duration: Infinity })
        }
      >
        {toastExample.trigger}
      </Button>
      <Toaster />
    </div>
  ),
  beforeEach: () => {
    toast.dismiss();
    return () => {
      toast.dismiss();
    };
  },
} satisfies Meta<typeof Toaster>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Success: Story = {
  play: playForImplementation(async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const portal = within(canvasElement.ownerDocument.body);
    await userEvent.click(await canvas.findByRole("button", { name: "保存" }));
    await waitFor(() => expect(portal.getByText("変更を保存しました")).toBeVisible());
  }),
};
