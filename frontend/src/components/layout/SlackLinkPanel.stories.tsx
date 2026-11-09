import { playForImplementation } from "../../../.storybook/play-for-implementation";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { delay, http, HttpResponse } from "msw";
import { expect, fn, screen, userEvent, waitFor, within } from "storybook/test";
import { SlackLinkPanel } from "./SlackLinkPanel";

const notLinked = http.get("*/api/slack/link", () =>
  HttpResponse.json({ configured: true, link: null }),
);
const linked = http.get("*/api/slack/link", () =>
  HttpResponse.json({
    configured: true,
    link: {
      team_id: "T1",
      team_name: "Zli",
      slack_user_id: "U1",
      slack_user_name: "alice.with.a.very.long.slack.display.name",
      linked_at: "2026-11-01T09:00:00Z",
    },
  }),
);
// 管理者がまだ ZIP を保存していない
const noExport = http.get("*/api/slack/imports", () =>
  HttpResponse.json({ export: null, channels: [], imports: [] }),
);
const exported = {
  id: "import-1",
  file_name: "Zli Slack export Jan 1 2024 - Nov 1 2026.zip",
  created_at: "2026-11-01T09:00:00Z",
};
const ownChannel = (overrides: Record<string, unknown>) => ({
  slack_channel_id: "C1",
  name: "design",
  is_private: false,
  is_archived: false,
  members: 8,
  creator: "U1",
  message_count: 120,
  suggested_name: "slack-design",
  name_taken: true,
  existing_channel: null,
  same_name_channel: { id: "channel-design", name: "design", is_private: false },
  targets: [{ id: "channel-design", name: "design", is_private: false }],
  ...overrides,
});
const ownChannels = [
  ownChannel({}),
  ownChannel({
    slack_channel_id: "G1",
    name: "secret",
    is_private: true,
    members: 3,
    message_count: 12,
    suggested_name: "secret",
    name_taken: false,
    same_name_channel: null,
    targets: [],
  }),
];
const run = {
  id: "run-1",
  status: "importing",
  file_name: exported.file_name,
  file_size: 1,
  team_id: "T1",
  analysis: null,
  total_messages: 132,
  processed_messages: 44,
  total_files: 0,
  processed_files: 0,
  pending_files: 0,
  failed_files: 0,
  error: null,
  created_at: "2026-11-02T09:00:00Z",
  finished_at: null,
  source_id: "import-1",
};
const meta = {
  title: "Features/Workspace/SlackLink",
  component: SlackLinkPanel,
  args: { navigateTo: fn() },
  parameters: {
    a11y: { test: "error" },
    router: { initialEntries: ["/settings?tab=slack"] },
  },
  decorators: [
    (Story) => (
      <div className="mx-auto max-w-2xl p-4">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof SlackLinkPanel>;
export default meta;
type Story = StoryObj<typeof meta>;

export const NotLinked: Story = {
  parameters: {
    msw: {
      handlers: [
        notLinked,
        http.post("*/api/slack/oauth/start", () =>
          HttpResponse.json({ authorize_url: "https://slack.com/oauth/v2/authorize?state=x" }),
        ),
      ],
    },
  },
  play: playForImplementation(async ({ args }) => {
    await userEvent.click(await screen.findByRole("button", { name: "Slack と連携" }));
    await waitFor(() =>
      expect(args.navigateTo).toHaveBeenCalledWith("https://slack.com/oauth/v2/authorize?state=x"),
    );
  }),
};

export const Linked: Story = {
  parameters: { msw: { handlers: [linked, noExport] } },
  play: playForImplementation(async () => {
    await userEvent.click(await screen.findByRole("button", { name: "連携を解除" }));
    // 開くアニメーションの途中は不可視と判定されるため、表示されきるまで待つ
    const dialog = await screen.findByRole("alertdialog");
    await waitFor(() => expect(dialog).toBeVisible());
  }),
};

/** SLACK_TEAM_ID を変えたあとに残っている、許可していないワークスペースとの連携 */
export const LinkedOtherWorkspace: Story = {
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/slack/link", () =>
          HttpResponse.json({
            configured: true,
            link: {
              team_id: "T0",
              team_name: "Old workspace",
              slack_user_id: "U1",
              slack_user_name: "alice",
              linked_at: "2026-11-01T09:00:00Z",
              team_allowed: false,
            },
          }),
        ),
        noExport,
      ],
    },
  },
  play: playForImplementation(async () => {
    await screen.findByText(
      "この Slack ワークスペースとの連携は、いまの設定では使えません。連携を解除してから、もう一度連携してください。",
    );
    await expect(screen.getByRole("button", { name: "連携を解除" })).toBeVisible();
    await expect(screen.queryByText("Slack から取り込む")).toBeNull();
  }),
};

export const NotConfigured: Story = {
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/slack/link", () => HttpResponse.json({ configured: false, link: null })),
      ],
    },
  },
  play: playForImplementation(async () => {
    await expect(await screen.findByText(/設定されていません/)).toBeVisible();
    await expect(screen.queryByRole("button", { name: "Slack と連携" })).not.toBeInTheDocument();
  }),
};

export const LoadError: Story = {
  parameters: {
    msw: {
      handlers: [http.get("*/api/slack/link", () => new HttpResponse(null, { status: 500 }))],
    },
  },
  play: playForImplementation(async () => {
    await expect(await screen.findByRole("alert")).toBeVisible();
  }),
};

export const ReturnedWithError: Story = {
  parameters: {
    router: { initialEntries: ["/settings?tab=slack&slack=error&reason=already_linked"] },
    msw: { handlers: [notLinked] },
  },
  play: playForImplementation(async () => {
    await expect(await screen.findByRole("alert")).toHaveTextContent(
      "別の Zoo アカウントと連携済み",
    );
  }),
};

export const ReturnedFromSlack: Story = {
  parameters: {
    router: { initialEntries: ["/settings?tab=slack&code=abc&state=signed"] },
    msw: {
      handlers: [
        noExport,
        // 連携前の状態を返す初期の読み込みが、完了処理より遅れて届く
        http.get("*/api/slack/link", async () => {
          await delay(300);
          return HttpResponse.json({ configured: true, link: null });
        }),
        http.post("*/api/slack/oauth/complete", () =>
          HttpResponse.json({
            configured: true,
            link: {
              team_id: "T1",
              team_name: "Zli",
              slack_user_id: "U1",
              slack_user_name: "alice",
              linked_at: "2026-11-01T09:00:00Z",
            },
          }),
        ),
      ],
    },
  },
  play: playForImplementation(async () => {
    await expect(await screen.findByText("Zli の alice として連携しています。")).toBeVisible();
    // 遅れて届いた連携前の状態で上書きしない
    await new Promise((resolve) => setTimeout(resolve, 500));
    await expect(screen.getByText("Zli の alice として連携しています。")).toBeVisible();
    await expect(screen.queryByRole("button", { name: "Slack と連携" })).not.toBeInTheDocument();
  }),
};

export const UnlinkError: Story = {
  parameters: {
    msw: {
      handlers: [
        linked,
        noExport,
        http.delete("*/api/slack/link", () =>
          HttpResponse.json({ error: "Slack token revocation failed" }, { status: 502 }),
        ),
      ],
    },
  },
  play: playForImplementation(async () => {
    await userEvent.click(await screen.findByRole("button", { name: "連携を解除" }));
    const dialog = await screen.findByRole("alertdialog");
    await waitFor(() => expect(dialog).toBeVisible());
    await userEvent.click(within(dialog).getByRole("button", { name: "連携を解除" }));
    await expect(await within(dialog).findByText(/連携は残っています/)).toBeVisible();
  }),
};

export const ReturnedFromSlackFailed: Story = {
  parameters: {
    router: { initialEntries: ["/settings?tab=slack&code=abc&state=expired"] },
    msw: {
      handlers: [
        notLinked,
        http.post("*/api/slack/oauth/complete", () =>
          HttpResponse.json({ error: "invalid_state" }, { status: 400 }),
        ),
      ],
    },
  },
  play: playForImplementation(async () => {
    await expect(await screen.findByRole("alert")).toHaveTextContent("有効期限");
    // 連携をやり直せる
    await expect(await screen.findByRole("button", { name: "Slack と連携" })).toBeVisible();
  }),
};

export const Mobile: Story = {
  parameters: Linked.parameters,
  globals: { viewport: { value: "mobile" } },
  play: playForImplementation(async () => {
    await expect(await screen.findByRole("button", { name: "連携を解除" })).toBeVisible();
  }),
};

let ownStarted = false;
export const OwnImport: Story = {
  beforeEach: () => {
    ownStarted = false;
  },
  parameters: {
    msw: {
      handlers: [
        linked,
        http.get("*/api/slack/imports", () =>
          HttpResponse.json({
            export: exported,
            channels: ownChannels,
            imports: ownStarted ? [run] : [],
          }),
        ),
        http.post("*/api/slack/imports", () => {
          ownStarted = true;
          return HttpResponse.json(run);
        }),
      ],
    },
  },
  play: playForImplementation(async () => {
    // 自分が作ったチャンネルだけが並び、同じ名前の参加中のチャンネルか、新しいチャンネルを選べる
    await userEvent.selectOptions(
      await screen.findByRole("combobox", { name: "#design の取り込み方" }),
      "existing",
    );
    await expect(screen.getByRole("combobox", { name: "#design の取り込み先" })).toHaveValue(
      "channel-design",
    );
    await userEvent.selectOptions(
      screen.getByRole("combobox", { name: "#secret の取り込み方" }),
      "create",
    );
    await userEvent.click(screen.getByRole("button", { name: "2 チャンネルを取り込む" }));
    await expect(await screen.findByRole("progressbar")).toHaveAttribute("aria-valuenow", "33");
  }),
};

let ownPolls = 0;
export const OwnImportChannelsChanged: Story = {
  beforeEach: () => {
    ownPolls = 0;
  },
  parameters: {
    msw: {
      handlers: [
        linked,
        // 取り込み中に管理者が新しい ZIP を保存し、自分が作ったチャンネルが増える
        http.get("*/api/slack/imports", () =>
          HttpResponse.json({
            export: exported,
            channels: ownPolls++ === 0 ? ownChannels.slice(0, 1) : ownChannels,
            imports: [run],
          }),
        ),
      ],
    },
  },
  play: playForImplementation(async () => {
    await expect(
      await screen.findByRole("combobox", { name: "#secret の取り込み方" }, { timeout: 5_000 }),
    ).toHaveValue("skip");
  }),
};

export const OwnImportNoChannels: Story = {
  parameters: {
    msw: {
      handlers: [
        linked,
        http.get("*/api/slack/imports", () =>
          HttpResponse.json({ export: exported, channels: [], imports: [] }),
        ),
      ],
    },
  },
  play: playForImplementation(async () => {
    await expect(await screen.findByText(/あなたが作ったチャンネルはありません/)).toBeVisible();
  }),
};

/** 以前の取り込み先が、見えないチャンネル (参加していない非公開チャンネル)。名前は出さず、そこ以外に取り込めないことだけを示す */
export const OwnImportHiddenTarget: Story = {
  parameters: {
    msw: {
      handlers: [
        linked,
        http.get("*/api/slack/imports", () =>
          HttpResponse.json({
            export: exported,
            channels: [
              ownChannel({
                slack_channel_id: "G1",
                name: "secret",
                is_private: true,
                existing_channel: null,
                existing_hidden: true,
                same_name_channel: null,
                targets: [],
              }),
            ],
            imports: [],
          }),
        ),
      ],
    },
  },
  play: playForImplementation(async () => {
    await expect(
      await screen.findByText(
        "以前、参加していないチャンネルに取り込みました。取り込めるのはそのチャンネルだけです。",
      ),
    ).toBeVisible();
    const action = screen.getByRole("combobox", { name: "#secret の取り込み方" });
    await expect(within(action).getByRole("option", { name: "新しいチャンネル" })).toBeDisabled();
    await expect(within(action).getByRole("option", { name: "既存のチャンネル" })).toBeDisabled();
  }),
};
