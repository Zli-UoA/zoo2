import { playForImplementation } from "../../../.storybook/play-for-implementation";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, screen, userEvent, waitFor, within } from "storybook/test";

import { StoryShell } from "../../../.storybook/shell";
import {
  DEFAULT_CURRENT_USER_ID,
  storyChannels,
  storyChannelUsers,
} from "../../../.storybook/fixtures";
import { ChannelHeader } from "./ChannelHeader";

const general = storyChannels.find((channel) => channel.id === "channel-general")!;
const secret = storyChannels.find((channel) => channel.id === "channel-secret")!;
const archived = storyChannels.find((channel) => channel.id === "channel-archive")!;
const thread = {
  ...general,
  id: "thread-channel",
  channel_name: "thread-release-plan",
  parent_channel_id: general.id,
  thread_title: "Release planning",
  thread_archived_at: null,
  thread_activity_at: general.created_at,
};
const memberCount = (channelId: string) =>
  storyChannelUsers.filter((row) => row.channel_id === channelId).length;

const meta = {
  title: "Features/Channels/Header",
  component: ChannelHeader,
  tags: ["autodocs"],
  parameters: { a11y: { test: "error" } },
  argTypes: {
    memberCount: { control: { type: "number", min: 0 } },
  },
  args: {
    channel: general,
    memberCount: memberCount(general.id),
  },
  decorators: [
    (Story) => (
      <StoryShell selectedChannelId={general.id}>
        <div className="w-full bg-background">
          <Story />
        </div>
      </StoryShell>
    ),
  ],
} satisfies Meta<typeof ChannelHeader>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {
  play: playForImplementation(async () => {
    const threads = await screen.findByRole("button", { name: "スレッド" });
    const members = await screen.findByRole("button", { name: /メンバー/ });
    const call = await screen.findByRole("button", { name: "通話を開始" });
    await expect(threads.getBoundingClientRect().x).toBeLessThan(members.getBoundingClientRect().x);
    await expect(members.getBoundingClientRect().x).toBeLessThan(call.getBoundingClientRect().x);
    await expect(threads).toHaveTextContent("スレッド");
  }),
};

export const ControlPanelOpen: Story = {
  play: playForImplementation(async () => {
    await userEvent.click(await screen.findByRole("button", { name: "チャンネル設定" }));
    await expect(await screen.findByRole("dialog")).toBeInTheDocument();
  }),
};

const importedAt = "2026-11-10 12:00:00.000";

/** 既存のチャンネルに、Slack から取り込んだ */
export const ImportedFromSlack: Story = {
  args: {
    channel: {
      ...general,
      imported_from_slack: true,
      slack_import_kind: "existing",
      slack_imported_at: importedAt,
    },
  },
  play: playForImplementation(async () => {
    await userEvent.click(await screen.findByRole("button", { name: "チャンネル設定" }));
    await expect(
      await within(await screen.findByRole("dialog")).findByText(
        /Slack から取り込み・既存（最後の取り込み 2026\/.+）/,
      ),
    ).toBeInTheDocument();
  }),
};

/** Slack から取り込んで、新しくチャンネルを作った */
export const ImportedFromSlackCreated: Story = {
  args: {
    channel: {
      ...general,
      imported_from_slack: true,
      slack_import_kind: "created",
      slack_imported_at: importedAt,
    },
  },
  play: playForImplementation(async () => {
    await userEvent.click(await screen.findByRole("button", { name: "チャンネル設定" }));
    await expect(
      await within(await screen.findByRole("dialog")).findByText(
        /Slack から取り込み・新規作成（最後の取り込み 2026\/.+）/,
      ),
    ).toBeInTheDocument();
  }),
};

/** 狭い画面でも、設定の見出しが 1 文字ずつ縦に並ばず、印は見出しの下へ折り返す */
export const ImportedFromSlackMobile: Story = {
  ...ImportedFromSlackCreated,
  globals: { viewport: { value: "mobile" } },
  play: playForImplementation(async () => {
    await userEvent.click(await screen.findByRole("button", { name: "チャンネル設定" }));
    const dialog = await screen.findByRole("dialog");
    const title = await within(dialog).findByText("チャンネル設定");
    await expect(title.getBoundingClientRect().width).toBeGreaterThan(100);
    const badge = await within(dialog).findByText(/Slack から取り込み・新規作成/);
    await expect(badge.getBoundingClientRect().top).toBeGreaterThanOrEqual(
      title.getBoundingClientRect().bottom,
    );
  }),
};

export const Private: Story = {
  args: {
    channel: secret,
    memberCount: memberCount(secret.id),
  },
};

export const Archived: Story = {
  parameters: { a11y: { test: "todo" } },
  args: {
    channel: archived,
    memberCount: memberCount(archived.id),
  },
};

export const LoadingMemberCount: Story = {
  args: {
    memberCount: null,
  },
};

export const NoDescription: Story = {
  args: {
    channel: { ...general, description: null },
  },
};

export const LongDescription: Story = {
  args: {
    channel: {
      ...general,
      description:
        "長い説明の表示確認用チャンネルです。目的・参加対象・運用ルールをまとめています。\n二行目も含めて崩れないことを確認します。",
    },
  },
};

export const Mobile: Story = {
  globals: { viewport: { value: "mobile" } },
};

export const WebhookSettingsOpen: Story = {
  play: playForImplementation(async () => {
    await userEvent.click(await screen.findByRole("button", { name: "チャンネル設定" }));
    const settings = await screen.findByRole("dialog", {
      name: "チャンネル設定",
    });
    await userEvent.click(within(settings).getByRole("button", { name: /^開く$/ }));
    await waitFor(async () => {
      await expect(screen.getByRole("dialog", { name: "Webhook設定" })).toBeVisible();
    });
    await waitFor(async () => {
      await expect(
        screen.queryByRole("dialog", { name: "チャンネル設定" }),
      ).not.toBeInTheDocument();
    });
    await userEvent.keyboard("{Escape}");
    await waitFor(async () => {
      await expect(screen.queryByRole("dialog", { name: "Webhook設定" })).not.toBeInTheDocument();
    });
  }),
};

export const ThreadChannel: Story = {
  args: { channel: thread, title: thread.thread_title, subtitle: "# general" },
  parameters: {
    a11y: { test: "error" },
    collections: {
      channels: [...storyChannels, thread],
      channelUsers: [
        ...storyChannelUsers,
        {
          user_id: DEFAULT_CURRENT_USER_ID,
          channel_id: thread.id,
          joined_at: thread.created_at,
        },
      ],
    },
  },
  play: playForImplementation(async () => {
    await expect(await screen.findByRole("button", { name: "スレッドをアーカイブ" })).toBeVisible();
    await expect(screen.queryByRole("button", { name: "チャンネル設定" })).not.toBeInTheDocument();
    await userEvent.click(await screen.findByRole("button", { name: "スレッドメニュー" }));
    await waitFor(async () => {
      await expect(screen.getByRole("menuitem", { name: "スレッドから退出" })).toBeVisible();
    });
  }),
};

export const ThreadChannelNotJoined: Story = {
  ...ThreadChannel,
  parameters: {
    ...ThreadChannel.parameters,
    collections: {
      channels: [...storyChannels, thread],
      channelUsers: storyChannelUsers.filter((row) => row.channel_id !== thread.id),
    },
  },
  play: playForImplementation(async () => {
    await expect(await screen.findByRole("button", { name: "参加" })).toBeVisible();
  }),
};
