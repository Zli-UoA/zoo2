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
// 作った非公開チャンネルがない
const noExport = http.get("*/api/slack/private-channels", () =>
  HttpResponse.json({ linked: true, channels: [] }),
);
const privateChannel = (overrides: Record<string, unknown>) => ({
  slack_channel_id: "G1",
  name: "design",
  is_archived: false,
  members: 8,
  status: "new",
  suggested_name: "design",
  name_taken: false,
  ...overrides,
});
const ownChannels = [
  privateChannel({}),
  privateChannel({
    slack_channel_id: "G2",
    name: "secret",
    members: 3,
    suggested_name: "slack-secret",
    name_taken: true,
  }),
];
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
        http.get("*/api/slack/private-channels", () =>
          HttpResponse.json({
            linked: true,
            channels: ownStarted
              ? [
                  privateChannel({ status: "running", channel_id: "c1", channel_name: "design" }),
                  ownChannels[1],
                ]
              : ownChannels,
          }),
        ),
        http.post("*/api/slack/private-channels/import", () => {
          ownStarted = true;
          return HttpResponse.json({
            imported: [{ slack_channel_id: "G1", channel_id: "c1", channel_name: "design" }],
          });
        }),
      ],
    },
  },
  play: playForImplementation(async () => {
    // 自分が作った非公開チャンネルが並び、取り込み先の名前を直して、1 つずつ取り込む
    const name = await screen.findByRole("textbox", { name: "#secret の取り込み先の名前" });
    await expect(name).toHaveValue("slack-secret");
    await userEvent.click(screen.getByRole("button", { name: "#design を取り込む" }));
    await expect(await screen.findByText(/取り込み中です（#design）/)).toBeVisible();
  }),
};

export const OwnImportDone: Story = {
  parameters: {
    msw: {
      handlers: [
        linked,
        http.get("*/api/slack/private-channels", () =>
          HttpResponse.json({
            linked: true,
            channels: [
              privateChannel({ status: "done", channel_id: "c1", channel_name: "design" }),
              privateChannel({ slack_channel_id: "G3", name: "old", status: "hidden" }),
            ],
          }),
        ),
      ],
    },
  },
  play: playForImplementation(async () => {
    await expect(await screen.findByText(/取り込み済みです（#design）/)).toBeVisible();
    await expect(screen.getByText(/名前は表示しません/)).toBeVisible();
  }),
};

export const OwnImportNoChannels: Story = {
  parameters: { msw: { handlers: [linked, noExport] } },
  play: playForImplementation(async () => {
    await expect(
      await screen.findByText(/あなたが作った非公開チャンネルはありません/),
    ).toBeVisible();
  }),
};

/** 以前の版で連携したので、非公開チャンネルを読むスコープがない。連携し直してもらう */
export const OwnImportNeedsRelink: Story = {
  parameters: {
    msw: {
      handlers: [
        linked,
        http.get("*/api/slack/private-channels", () =>
          HttpResponse.json({ linked: true, reason: "missing_scope", channels: [] }),
        ),
      ],
    },
  },
  play: playForImplementation(async () => {
    await expect(await screen.findByText(/非公開チャンネルを読む権限がありません/)).toBeVisible();
  }),
};
