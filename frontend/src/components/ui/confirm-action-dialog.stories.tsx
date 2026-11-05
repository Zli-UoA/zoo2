import { playForImplementation } from "../../../.storybook/play-for-implementation";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, fn, userEvent, within } from "storybook/test";
import { ConfirmActionDialog } from "./confirm-action-dialog";
const meta = {
  title: "Components/ConfirmActionDialog",
  component: ConfirmActionDialog,
  parameters: { a11y: { test: "error" } },
  args: {
    open: true,
    onOpenChange: fn(),
    onConfirm: fn(),
    title: "Appを削除",
    description: "このAppと関連する設定を削除します。この操作は取り消せません。",
    confirmLabel: "削除",
  },
} satisfies Meta<typeof ConfirmActionDialog>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Default: Story = {
  play: playForImplementation(async ({ canvasElement, args }) => {
    const portal = within(canvasElement.ownerDocument.body);
    await userEvent.click(await portal.findByRole("button", { name: "削除" }));
    await expect(args.onConfirm).toHaveBeenCalledTimes(1);
    await userEvent.click(portal.getByRole("button", { name: "キャンセル" }));
    await expect(args.onOpenChange).toHaveBeenCalledWith(false);
  }),
};
export const Pending: Story = { args: { pending: true } };
export const Failed: Story = {
  args: { error: "変更を保存できませんでした。もう一度お試しください。" },
};
