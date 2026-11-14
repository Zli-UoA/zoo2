import { playForImplementation } from "../../../.storybook/play-for-implementation";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, fn, userEvent, waitFor, within } from "storybook/test";
import { AppEncryptedChannelConsent } from "./AppEncryptedChannelConsent";
const meta = {
  title: "Features/Apps/EncryptedChannelConsent",
  component: AppEncryptedChannelConsent,
  parameters: { a11y: { test: "error" } },
  args: {
    open: true,
    appName: "Echo Bot",
    channelName: "secret",
    supported: true,
    onConfirm: fn(),
    onOpenChange: fn(),
  },
} satisfies Meta<typeof AppEncryptedChannelConsent>;
export default meta;
type Story = StoryObj<typeof meta>;
/** The add button stays disabled until the member explicitly agrees. */
export const Default: Story = {
  play: playForImplementation(async ({ canvasElement, args }) => {
    const portal = within(canvasElement.ownerDocument.body);
    const confirm = await portal.findByRole("button", { name: "許可して追加" });
    await expect(confirm).toBeDisabled();
    await userEvent.click(
      portal.getByRole("checkbox", {
        name: "内容を理解し、Echo Bot がこのチャンネルのメッセージを読むことを許可します",
      }),
    );
    await expect(confirm).toBeEnabled();
    await userEvent.click(confirm);
    await expect(args.onConfirm).toHaveBeenCalledTimes(1);
  }),
};
export const Pending: Story = { args: { pending: true } };
export const NotReady: Story = {
  args: {
    error:
      "この App は暗号化の準備（端末の登録）がまだ済んでいません。App を起動してから、もう一度試してください。",
  },
};
export const Unsupported: Story = {
  args: { supported: false },
  play: playForImplementation(async ({ canvasElement }) => {
    const portal = within(canvasElement.ownerDocument.body);
    const notice = await portal.findByText(/この App は暗号化チャンネルに対応していません/);
    // findByText waits for the element only; wait for the dialog's open transition too.
    await waitFor(() => expect(notice).toBeVisible());
    await expect(portal.queryByRole("button", { name: "許可して追加" })).not.toBeInTheDocument();
  }),
};
export const Mobile: Story = { globals: { viewport: { value: "mobile" } } };
