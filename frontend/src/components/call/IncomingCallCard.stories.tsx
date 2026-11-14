import { playForImplementation } from "../../../.storybook/play-for-implementation";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, fn, screen, userEvent } from "storybook/test";

import { StoryShell } from "../../../.storybook/shell";
import { IncomingCallCard } from "./IncomingCallCard";

const meta = {
  title: "Features/Calls/IncomingCall",
  component: IncomingCallCard,
  tags: ["autodocs"],
  parameters: { a11y: { test: "error" } },
  args: {
    inviterName: "Bob",
    channelLabel: "Bob",
    isDm: true,
    onAccept: fn(),
    onDecline: fn(),
  },
  decorators: [
    (Story) => (
      <StoryShell>
        <div className="relative h-40 w-full bg-background">
          <Story />
        </div>
      </StoryShell>
    ),
  ],
} satisfies Meta<typeof IncomingCallCard>;

export default meta;
type Story = StoryObj<typeof meta>;

/** DM の着信。応答と拒否の操作が、それぞれ呼ばれる。 */
export const DirectMessage: Story = {
  play: playForImplementation(async ({ args }) => {
    await expect(await screen.findByRole("alertdialog")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "応答" }));
    await expect(args.onAccept).toHaveBeenCalledTimes(1);
    await userEvent.click(screen.getByRole("button", { name: "拒否" }));
    await expect(args.onDecline).toHaveBeenCalledTimes(1);
  }),
};

/** チャンネルの招待。チャンネル名を出す。 */
export const ChannelInvite: Story = {
  args: { inviterName: "Alice", channelLabel: "#general", isDm: false },
};

export const LongNames: Story = {
  args: {
    inviterName: "とても長い表示名のユーザーとても長い表示名のユーザー",
    channelLabel: "#とても長いチャンネル名とても長いチャンネル名とても長いチャンネル名",
    isDm: false,
  },
};

/** 応答中は、二重に押せない。 */
export const Busy: Story = {
  args: { busy: true },
  play: playForImplementation(async () => {
    await expect(await screen.findByRole("button", { name: "応答" })).toBeDisabled();
  }),
};

/** 応答しようとしたときに、もう終わっていた。 */
export const Ended: Story = {
  args: { error: "この着信はすでに終了しました" },
  play: playForImplementation(async () => {
    await expect(await screen.findByRole("alert")).toHaveTextContent("すでに終了");
  }),
};
