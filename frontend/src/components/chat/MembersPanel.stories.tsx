import { avatarUsers, userAvatarHandler, userAvatarUrl } from "../../../.storybook/avatar-fixtures";
import { playForImplementation } from "../../../.storybook/play-for-implementation";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { http, HttpResponse } from "msw";
import { expect, screen, userEvent, waitFor } from "storybook/test";

import { storyChannels, storyCollections, storyChannelUsers } from "../../../.storybook/fixtures";
import { MembersPanel } from "./MembersPanel";

const general = storyChannels.find(
  (channel) => channel.id === "00000000-0000-4000-8000-000000000001",
)!;
const secret = storyChannels.find((channel) => channel.id === "channel-secret")!;
const archived = storyChannels.find((channel) => channel.id === "channel-archive")!;

const meta = {
  title: "Features/Channels/Members",
  component: MembersPanel,
  tags: ["autodocs"],
  parameters: { a11y: { test: "todo" } },
  args: {
    channel: general,
  },
  decorators: [
    (Story) => (
      <div className="h-[640px] w-80 bg-background">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof MembersPanel>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};

export const AddCandidates: Story = {
  play: playForImplementation(async () => {
    const input = await screen.findByRole("combobox", { name: "メンバーを追加" });
    await userEvent.click(input);
    await expect(screen.getByRole("listbox", { name: "メンバー候補" })).toBeVisible();
  }),
};

export const Archived: Story = {
  args: {
    channel: archived,
  },
};

export const PrivateNotJoined: Story = {
  args: {
    channel: secret,
  },
  parameters: {
    collections: {
      ...storyCollections,
      channelUsers: storyChannelUsers.filter(
        (row) => !(row.channel_id === secret.id && row.user_id === "user-alice"),
      ),
    },
  },
};

export const UserAvatar: Story = {
  parameters: {
    collections: { ...storyCollections, users: avatarUsers },
    msw: { handlers: [userAvatarHandler] },
  },
  play: playForImplementation(async ({ canvasElement }) => {
    await waitFor(async () => {
      const image = canvasElement.querySelector<HTMLImageElement>(`img[src="${userAvatarUrl}"]`);
      await expect(image?.naturalWidth).toBeGreaterThan(0);
    });
  }),
};

/**
 * 暗号化チャンネルで、新しいメッセージを読めないメンバーに警告を出す。user-alice は暗号化を設定して
 * 端末があり、user-bob は暗号化を設定していない（配布先のスナップショットにクロス署名鍵がない）。
 */
export const EncryptedUnreadableMember: Story = {
  args: { channel: { ...secret, encrypted: true, membership_epoch: "1" } },
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/e2ee/channels/:id/recipients", () =>
          HttpResponse.json({
            channel_id: secret.id,
            membership_epoch: "1",
            user_ids: ["user-alice", "user-bob"],
            device_keys: { "user-alice": { ALICE1: { device_id: "ALICE1" } }, "user-bob": {} },
            master_keys: { "user-alice": { keys: {} } },
            self_signing_keys: { "user-alice": { keys: {} } },
            user_signing_keys: {},
            signatures: [],
            tofu_pins: {},
            next_after: null,
          }),
        ),
      ],
    },
  },
  play: playForImplementation(async () => {
    const warnings = await screen.findAllByText("暗号化未設定");
    await expect(warnings).toHaveLength(1);
    await expect(warnings[0]).toBeVisible();
  }),
};
