import { playForImplementation } from "../../../.storybook/play-for-implementation";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { http, HttpResponse } from "msw";
import { expect, fn, screen, userEvent, within } from "storybook/test";

import { StoryShell } from "../../../.storybook/shell";
import { storyChannels } from "../../../.storybook/fixtures";
import { ChannelThreadsDialog } from "./ChannelThreadsDialog";

const channel = storyChannels.find((row) => row.id === "channel-general")!;

const meta = {
  title: "Features/Channels/Threads",
  component: ChannelThreadsDialog,
  tags: ["autodocs"],
  parameters: {
    a11y: { test: "error" },
    msw: {
      handlers: [
        http.get("*/api/channels/channel-general/threads", () =>
          HttpResponse.json({
            items: [
              {
                channel_id: "thread-release-plan",
                title: "Release planning",
                archived_at: null,
                activity_at: "2026-11-13T02:00:00Z",
                member_count: 3,
                is_member: true,
                last_message: "Review the final checklist",
              },
            ],
          }),
        ),
      ],
    },
  },
  args: { channel, open: true, onOpenChange: fn() },
  render: (args) => (
    <StoryShell selectedChannelId={channel.id}>
      <ChannelThreadsDialog {...args} onOpenChange={() => {}} />
    </StoryShell>
  ),
} satisfies Meta<typeof ChannelThreadsDialog>;

export default meta;
type Story = StoryObj<typeof meta>;

export const ActiveThreads: Story = {
  args: { channel, open: true, onOpenChange: fn() },
  play: playForImplementation(async () => {
    const dialog = await screen.findByRole("dialog", { name: "スレッド" });
    await expect(
      await within(dialog).findByRole("button", { name: /Release planning/ }),
    ).toBeVisible();
  }),
};

export const CreateThread: Story = {
  args: { channel, open: true, onOpenChange: fn() },
  play: playForImplementation(async () => {
    await userEvent.click(await screen.findByRole("button", { name: "スレッドを作成" }));
    await userEvent.type(
      await screen.findByPlaceholderText("スレッドのタイトル"),
      "Release planning",
    );
    await userEvent.type(
      screen.getByPlaceholderText("最初の投稿を書く"),
      "Review the release checklist",
    );
    await expect(await screen.findByRole("button", { name: "スレッドを作成" })).toBeEnabled();
  }),
};
