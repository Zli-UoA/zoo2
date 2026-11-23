import { avatarUsers, userAvatarHandler, userAvatarUrl } from "../../../.storybook/avatar-fixtures";
import { playForImplementation } from "../../../.storybook/play-for-implementation";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { http, HttpResponse } from "msw";
import { expect, fn, screen, userEvent, waitFor } from "storybook/test";

import { storyChannels, storyCollections, storyChannelUsers } from "../../../.storybook/fixtures";
import { ChannelMembersDialog } from "./ChannelMembersDialog";

const general = storyChannels.find(
  (channel) => channel.id === "00000000-0000-4000-8000-000000000001",
)!;
const secret = storyChannels.find((channel) => channel.id === "channel-secret")!;
const archived = storyChannels.find((channel) => channel.id === "channel-archive")!;

const meta = {
  title: "Features/Channels/Members",
  component: ChannelMembersDialog,
  tags: ["autodocs"],
  parameters: { a11y: { test: "todo" } },
  args: {
    channel: general,
    open: true,
    onOpenChange: fn(),
  },
} satisfies Meta<typeof ChannelMembersDialog>;

export default meta;
type Story = StoryObj<typeof meta>;

/**
 * スレッドのメンバーと同じ、画面中央のダイアログ。見出しに人数、説明にチャンネル名を出し、
 * Escape で閉じる（閉じる操作は呼び出し側の `onOpenChange` に渡る）。
 */
export const Default: Story = {
  play: playForImplementation(async ({ args }) => {
    // 開くアニメーションが終わるまでは透明なので、見えるまで待つ。
    const dialog = await screen.findByRole("dialog", { name: /メンバー/ });
    await waitFor(() => expect(dialog).toBeVisible());
    await expect(await screen.findByText("Alice")).toBeVisible();
    await expect(screen.getByText(`#${general.channel_name}`)).toBeVisible();
    await userEvent.keyboard("{Escape}");
    await waitFor(() => expect(args.onOpenChange).toHaveBeenCalledWith(false, expect.anything()));
  }),
};

export const AddCandidates: Story = {
  play: playForImplementation(async ({ args }) => {
    const input = await screen.findByRole("combobox", { name: "メンバーを追加" });
    await userEvent.click(input);
    await expect(screen.getByRole("listbox", { name: "メンバー候補" })).toBeVisible();
    // 候補を開いているあいだの Escape は候補だけを閉じ、ダイアログは閉じない。
    await userEvent.keyboard("{Escape}");
    await expect(screen.queryByRole("listbox", { name: "メンバー候補" })).not.toBeInTheDocument();
    await expect(screen.getByRole("dialog", { name: /メンバー/ })).toBeVisible();
    await expect(args.onOpenChange).not.toHaveBeenCalled();
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
  play: playForImplementation(async () => {
    await waitFor(async () => {
      // ダイアログは canvas の外（body 直下）へポータルで描画する。
      const image = document.querySelector<HTMLImageElement>(`img[src="${userAvatarUrl}"]`);
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
            device_keys: {
              "user-alice": {
                ALICE1: {
                  device_id: "ALICE1",
                  signatures: { "user-alice": { "ed25519:ALICESSK": "sig" } },
                },
              },
              "user-bob": {},
            },
            master_keys: { "user-alice": { keys: { "ed25519:ALICEMSK": "ALICEMSK" } } },
            self_signing_keys: { "user-alice": { keys: { "ed25519:ALICESSK": "ALICESSK" } } },
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

/** 狭幅でも全画面に切り替えず、同じダイアログを画面中央に出す。 */
export const Mobile: Story = {
  globals: { viewport: { value: "mobile" } },
  parameters: { a11y: { test: "error" } },
  play: playForImplementation(async () => {
    const dialog = await screen.findByRole("dialog", { name: /メンバー/ });
    await waitFor(() => expect(dialog).toBeVisible());
    await expect(screen.getByRole("button", { name: "退出" })).toBeVisible();
  }),
};

export const MobilePublicNotJoined: Story = {
  globals: { viewport: { value: "mobile" } },
  parameters: {
    a11y: { test: "error" },
    collections: {
      ...storyCollections,
      channelUsers: storyChannelUsers.filter(
        (row) => !(row.channel_id === general.id && row.user_id === "user-alice"),
      ),
    },
  },
  play: playForImplementation(async () => {
    const join = await screen.findByRole("button", { name: "参加" });
    await waitFor(() => expect(join).toBeVisible());
  }),
};
