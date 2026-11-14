import { playForImplementation } from "../../../.storybook/play-for-implementation";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, fn, userEvent, within } from "storybook/test";
import { EncryptChannelConfirm } from "./EncryptChannelDialog";

const meta = {
  title: "Features/Chat/EncryptChannelDialog",
  component: EncryptChannelConfirm,
  parameters: { a11y: { test: "error" } },
  args: {
    open: true,
    isDm: false,
    onConfirm: fn(),
    onOpenChange: fn(),
  },
} satisfies Meta<typeof EncryptChannelConfirm>;
export default meta;
type Story = StoryObj<typeof meta>;

/** 非公開チャンネル。これまでのメッセージ、連合済みの投稿、連携の扱いと、戻せないことを示してから切り替える。 */
export const Channel: Story = {
  play: playForImplementation(async ({ canvasElement, args }) => {
    const portal = within(canvasElement.ownerDocument.body);
    await expect(await portal.findByText("このチャンネルを暗号化しますか？")).toBeInTheDocument();
    await expect(portal.getByText(/Home や ActivityPub にすでに出した投稿/)).toBeInTheDocument();
    await expect(portal.getByText("暗号化はあとから解除できません。")).toBeInTheDocument();
    await userEvent.click(portal.getByRole("button", { name: "暗号化する" }));
    await expect(args.onConfirm).toHaveBeenCalledTimes(1);
  }),
};

/** DM には、チャンネルだけの項目（連合と連携）を出さない。 */
export const Dm: Story = {
  args: { isDm: true },
  play: playForImplementation(async ({ canvasElement }) => {
    const portal = within(canvasElement.ownerDocument.body);
    await expect(await portal.findByText("この DM を暗号化しますか？")).toBeInTheDocument();
    await expect(portal.queryByText(/Home や ActivityPub/)).not.toBeInTheDocument();
  }),
};

export const Pending: Story = { args: { pending: true } };

/** App や Webhook が残っているときの 409 E2EE_CONVERSION_BLOCKED の文言。 */
export const Blocked: Story = {
  args: {
    error:
      "App（Bot）がメンバーにいます。先にチャンネルから外してください。 Incoming Webhook があります。先に削除してください。",
  },
};

export const Mobile: Story = { globals: { viewport: { value: "mobile" } } };
