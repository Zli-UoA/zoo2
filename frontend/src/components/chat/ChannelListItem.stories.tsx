import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, fn } from "storybook/test";

import { playForImplementation } from "../../../.storybook/play-for-implementation";

import { storyChannels } from "../../../.storybook/fixtures";
import { CallContext } from "../../lib/call/call-context";
import type { CallContextValue } from "../../lib/call/types";
import { ChannelListItem } from "./ChannelListItem";

const channel = storyChannels.find((item) => item.channel_name === "design")!;

const meta = {
  title: "Features/Channels/ListItem",
  component: ChannelListItem,
  tags: ["autodocs"],
  parameters: { a11y: { test: "error" } },
  args: {
    channel,
    active: false,
    unread: false,
    onOpen: fn(),
  },
  decorators: [
    (Story) => (
      <div className="w-72 bg-sidebar p-2">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof ChannelListItem>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};

export const Active: Story = {
  args: { active: true },
};

export const Unread: Story = {
  args: { unread: true },
};

/** 通常の一覧とピン留めの両方で、NSFW の印を出す。 */
export const Nsfw: Story = {
  args: { channel: { ...channel, is_nsfw: true } },
};

/** 通話中のチャンネルには、ピン留めの行でもヘッドホンのアイコンを出す。 */
/** ドラッグ中の挿入位置。行の上端に移動先の線を出す。 */
export const DropIndicator: Story = {
  args: { dropIndicator: "before", draggable: true },
  play: playForImplementation(async ({ canvasElement }) => {
    await expect(canvasElement.querySelector('[data-drop-indicator="before"]')).toBeVisible();
  }),
};

export const ActiveCall: Story = {
  decorators: [
    (Story) => (
      <CallContext.Provider
        value={{ activeCalls: new Map([[channel.id, 2]]) } as unknown as CallContextValue}
      >
        <Story />
      </CallContext.Provider>
    ),
  ],
};
