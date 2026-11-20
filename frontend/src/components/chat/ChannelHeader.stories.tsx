import { playForImplementation } from "../../../.storybook/play-for-implementation";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { http, HttpResponse } from "msw";
import { expect, screen, userEvent, waitFor, within } from "storybook/test";

import { StoryShell } from "../../../.storybook/shell";
import { storyChannels, storyChannelUsers } from "../../../.storybook/fixtures";
import { ChannelHeader } from "./ChannelHeader";

const general = storyChannels.find(
  (channel) => channel.id === "00000000-0000-4000-8000-000000000001",
)!;
const secret = storyChannels.find((channel) => channel.id === "channel-secret")!;
const archived = storyChannels.find((channel) => channel.id === "channel-archive")!;
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

export const Default: Story = {};

const pinnedAt = "2026-01-03 09:00:00.000";
const pinnedRow = {
  user_id: "user-alice",
  channel_id: general.id,
  position: 0,
  archived: false,
  created_at: pinnedAt,
  updated_at: pinnedAt,
};

/** ピン留め済みのチャンネルでは、解除のボタンになる。 */
export const Pinned: Story = {
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/channel-pins", () => HttpResponse.json({ pins: [pinnedRow], version: 1 })),
      ],
    },
  },
  play: playForImplementation(async () => {
    const button = await screen.findByRole("button", { name: "ピン留めを解除" });
    await expect(button).toHaveAttribute("aria-pressed", "true");
  }),
};

const pinRequests: unknown[] = [];

/** 上限に当たったときも、押した操作が API に届き、状態は変わらない。 */
export const PinLimit: Story = {
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/channel-pins", () => HttpResponse.json({ pins: [], version: 0 })),
        http.post("*/api/channel-pins", async ({ request }) => {
          pinRequests.push(await request.json());
          return HttpResponse.json(
            {
              error: "limit",
              code: "CHANNEL_PINS_LIMIT_EXCEEDED",
              limit: 50,
            },
            { status: 400 },
          );
        }),
      ],
    },
  },
  play: playForImplementation(async () => {
    pinRequests.length = 0;
    const button = await screen.findByRole("button", { name: "ピン留め" });
    await userEvent.click(button);
    await waitFor(() => expect(pinRequests).toHaveLength(1));
    await expect(pinRequests[0]).toMatchObject({ channel_id: general.id });
    await expect(button).toHaveAttribute("aria-pressed", "false");
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

/** 非公開チャンネルの設定から、暗号化に切り替える（#367）。公開チャンネルと暗号化済みのチャンネルには出さない。 */
export const PrivateEncryptSetting: Story = {
  args: {
    channel: secret,
    memberCount: memberCount(secret.id),
  },
  play: playForImplementation(async () => {
    await userEvent.click(await screen.findByRole("button", { name: "チャンネル設定" }));
    await userEvent.click(await screen.findByRole("button", { name: "暗号化を有効にする" }));
    await expect(await screen.findByText("このチャンネルを暗号化しますか？")).toBeInTheDocument();
  }),
};

const archivedPin = {
  user_id: "user-alice",
  channel_id: archived.id,
  position: 0,
  archived: true,
  created_at: pinnedAt,
  updated_at: pinnedAt,
};
const archivedPinFetches: string[] = [];
const archivedUnpinRequests: string[] = [];

/** アーカイブ中でピン留めされていないチャンネルは、ピン留めできない（サーバーが拒否する）。 */
export const Archived: Story = {
  parameters: {
    a11y: { test: "todo" },
    msw: {
      handlers: [
        http.get("*/api/channel-pins", () => {
          archivedPinFetches.push("GET");
          return HttpResponse.json({ pins: [], version: 0 });
        }),
      ],
    },
  },
  beforeEach: () => {
    archivedPinFetches.length = 0;
  },
  args: {
    channel: archived,
    memberCount: memberCount(archived.id),
  },
  play: playForImplementation(async () => {
    // ピンの状態がそろってから確認する。そろう前はボタンが出ないので、偽陽性になる。
    await waitFor(() => expect(archivedPinFetches.length).toBeGreaterThan(0));
    await expect(screen.queryByRole("button", { name: "ピン留め" })).not.toBeInTheDocument();
    await expect(screen.queryByRole("button", { name: "ピン留めを解除" })).not.toBeInTheDocument();
  }),
};

/** アーカイブ中でも、ピン留めしたままなら解除できる（ピンはアーカイブ後も残る）。 */
export const PinnedArchived: Story = {
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/channel-pins", () =>
          HttpResponse.json({ pins: [archivedPin], version: 1 }),
        ),
        http.delete("*/api/channel-pins/:channelId", ({ params }) => {
          archivedUnpinRequests.push(String(params.channelId));
          return HttpResponse.json({
            data: { removed: true, version: 2, pins: [] },
            txid: "2",
          });
        }),
      ],
    },
  },
  beforeEach: () => {
    archivedUnpinRequests.length = 0;
  },
  args: {
    channel: archived,
    memberCount: memberCount(archived.id),
  },
  play: playForImplementation(async () => {
    const button = await screen.findByRole("button", { name: "ピン留めを解除" });
    await expect(button).toHaveAttribute("aria-pressed", "true");
    await userEvent.click(button);
    await waitFor(() => expect(archivedUnpinRequests).toEqual([archived.id]));
    await waitFor(() =>
      expect(screen.queryByRole("button", { name: "ピン留めを解除" })).not.toBeInTheDocument(),
    );
    // アーカイブ中は新しくピン留めできないので、解除後はピンのボタンを出さない。
    await expect(screen.queryByRole("button", { name: "ピン留め" })).not.toBeInTheDocument();
  }),
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
    const settings = await screen.findByRole("dialog", { name: "チャンネル設定" });
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

export const Encrypted: Story = {
  args: {
    channel: { ...secret, encrypted: true, membership_epoch: "1" },
    memberCount: memberCount(secret.id),
  },
  play: playForImplementation(async () => {
    // ヘッダーには印を出さず（読み上げ用の文字だけ）、チャンネル設定に「暗号化」の印を出す。
    await expect(await screen.findByText("暗号化")).toHaveClass("sr-only");
    await userEvent.click(await screen.findByRole("button", { name: "チャンネル設定" }));
    const dialog = await screen.findByRole("dialog");
    // 開くときのアニメーションの途中は、中身がまだ見えないことがあるので、見えるまで待つ。
    await waitFor(() => expect(within(dialog).getByText("暗号化")).toBeVisible());
  }),
};

/** 暗号化 DM。DM はチャンネル設定を開けないので、DM のアイコンに鍵を重ねて見分けられるようにする。 */
export const EncryptedDm: Story = {
  args: {
    channel: { ...secret, is_dm: true, encrypted: true, membership_epoch: "1" },
    memberCount: memberCount(secret.id),
  },
  play: playForImplementation(async ({ canvasElement }) => {
    await waitFor(() =>
      expect(canvasElement.querySelector("svg.lucide-lock-keyhole")).not.toBeNull(),
    );
  }),
};
