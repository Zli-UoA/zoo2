import { playForImplementation } from "../../../../.storybook/play-for-implementation";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, within } from "storybook/test";

import { storyUsers } from "../../../../.storybook/fixtures";
import { EncryptionRecipientsNotice } from "./EncryptionRecipientsNotice";

const [alice, bob, carol] = storyUsers;

const meta = {
  title: "Features/Chat/EncryptionRecipientsNotice",
  component: EncryptionRecipientsNotice,
  parameters: { a11y: { test: "error" } },
  args: {
    users: storyUsers,
    summary: {
      epoch: 3,
      newPins: [],
      undeliverable: [],
      deviceCount: 4,
      userCount: 2,
      userIds: [alice!.id, bob!.id],
    },
  },
} satisfies Meta<typeof EncryptionRecipientsNotice>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {
  play: playForImplementation(async ({ canvasElement }) => {
    await expect(
      within(canvasElement).getByText("2 人の 4 台の端末あてに暗号化しています。"),
    ).toBeVisible();
  }),
};

export const Undeliverable: Story = {
  args: {
    summary: {
      epoch: 3,
      newPins: [],
      undeliverable: [
        { userId: carol!.id, reason: "notSetUp" },
        { userId: bob!.id, reason: "needsReverification" },
      ],
      deviceCount: 1,
      userCount: 1,
      userIds: [alice!.id, bob!.id, carol!.id],
    },
  },
  play: playForImplementation(async ({ canvasElement }) => {
    await expect(
      within(canvasElement).getByText("次の人は新しいメッセージを読めません"),
    ).toBeVisible();
  }),
};
