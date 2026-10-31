import type { Meta, StoryObj } from "@storybook/react-vite";
import { toast } from "sonner";
import { expect, screen, userEvent, waitFor } from "storybook/test";
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
          toast.success("変更を保存しました", { id: "storybook-toast", duration: Infinity })
        }
      >
        保存
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
  play: async () => {
    await userEvent.click(screen.getByRole("button", { name: "保存" }));
    await waitFor(() => expect(screen.getByText("変更を保存しました")).toBeVisible());
  },
};
