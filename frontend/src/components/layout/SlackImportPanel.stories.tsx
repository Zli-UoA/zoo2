import { playForImplementation } from "../../../.storybook/play-for-implementation";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { delay, http, HttpResponse } from "msw";
import { expect, screen, userEvent, waitFor, within } from "storybook/test";
import type { SlackImport } from "../../lib/api/slackImport";
import { SlackImportPanel } from "./SlackImportPanel";

const base: SlackImport = {
  id: "import-1",
  status: "ready",
  file_name: "Zli Slack export Jan 1 2024 - Nov 1 2026.zip",
  file_size: 12_345_678,
  team_id: "T1",
  analysis: {
    team_id: "T1",
    user_count: 42,
    channels: [
      {
        slack_channel_id: "C1",
        creator: "U1",
        name: "general",
        is_private: false,
        is_archived: false,
        members: 42,
        message_count: 12_034,
        suggested_name: "slack-general",
        name_taken: true,
        existing_channel: null,
      },
      {
        slack_channel_id: "C2",
        creator: "U2",
        name: "random",
        is_private: false,
        is_archived: false,
        members: 30,
        message_count: 860,
        suggested_name: "random",
        name_taken: false,
        existing_channel: { id: "channel-random", name: "random" },
      },
      {
        slack_channel_id: "G1",
        creator: "U3",
        name: "very-long-private-channel-name-for-the-infrastructure-working-group",
        is_private: true,
        is_archived: false,
        members: 5,
        message_count: 31,
        suggested_name: "very-long-private-channel-name-for-the-infrastructure-working-group",
        name_taken: false,
        existing_channel: null,
      },
      {
        slack_channel_id: "C3",
        creator: "U1",
        name: "design",
        is_private: false,
        is_archived: false,
        members: 8,
        message_count: 120,
        suggested_name: "design",
        name_taken: false,
        existing_channel: null,
        deleted_channel: "slack-design",
      },
    ],
  },
  total_messages: 12_925,
  processed_messages: 0,
  total_files: 0,
  processed_files: 0,
  pending_files: 0,
  failed_files: 0,
  error: null,
  created_at: "2026-11-01T09:00:00Z",
  finished_at: null,
  source_id: null,
};

/** 保存した ZIP (base) からの取り込み */
const run = (overrides: Partial<SlackImport>): SlackImport => ({
  ...base,
  id: "run-1",
  status: "importing",
  analysis: null,
  source_id: base.id,
  ...overrides,
});

const list = (...imports: SlackImport[]) =>
  http.get("*/api/admin/slack-imports", () => HttpResponse.json({ imports }));

const meta = {
  title: "Features/Workspace/SlackImport",
  component: SlackImportPanel,
  parameters: { a11y: { test: "error" } },
  decorators: [
    (Story) => (
      <div className="mx-auto max-w-2xl p-4">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof SlackImportPanel>;
export default meta;
type Story = StoryObj<typeof meta>;

export const Empty: Story = {
  parameters: { msw: { handlers: [list()] } },
  play: playForImplementation(async () => {
    await expect(
      await screen.findByRole("button", { name: "ZIP を選んでアップロード" }),
    ).toBeEnabled();
  }),
};

export const Loading: Story = {
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/admin/slack-imports", async () => {
          await delay("infinite");
          return HttpResponse.json({ imports: [] });
        }),
      ],
    },
  },
};

export const LoadError: Story = {
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/admin/slack-imports", () =>
          HttpResponse.json(
            { error: "Only workspace admins can import from Slack" },
            { status: 403 },
          ),
        ),
      ],
    },
  },
  play: playForImplementation(async () => {
    await expect(await screen.findByRole("alert")).toHaveTextContent("管理者だけ");
  }),
};

export const Mapping: Story = {
  parameters: { msw: { handlers: [list(base)] } },
  play: playForImplementation(async () => {
    // 取り込むチャンネルは管理者が選ぶので、初めはどれも取り込まない
    const start = await screen.findByRole("button", { name: "0 チャンネルを取り込む" });
    await expect(start).toBeDisabled();
    await userEvent.selectOptions(
      screen.getByRole("combobox", { name: "#general の取り込み方" }),
      "create",
    );
    await expect(
      screen.getByRole("textbox", { name: "#general の新しいチャンネル名" }),
    ).toHaveValue("slack-general");
    await expect(screen.getByRole("button", { name: "1 チャンネルを取り込む" })).toBeEnabled();
    // 取り込み先が削除済みの #design は取り込まない
    await expect(screen.getByText(/#slack-design が削除されている/)).toBeVisible();
    // 以前取り込んだ #random は同じ取り込み先に固定する
    await expect(screen.getByText("取り込み先: #random")).toBeVisible();
    await userEvent.selectOptions(
      screen.getByRole("combobox", { name: "#random の取り込み方" }),
      "existing",
    );
    await expect(screen.getByRole("combobox", { name: "#random の取り込み先" })).toBeDisabled();
  }),
};

let started = false;
export const Started: Story = {
  beforeEach: () => {
    started = false;
  },
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/admin/slack-imports", () =>
          HttpResponse.json({ imports: started ? [run({}), base] : [base] }),
        ),
        http.post("*/api/admin/slack-imports/import-1/start", () => {
          started = true;
          return HttpResponse.json(run({}));
        }),
      ],
    },
  },
  play: playForImplementation(async () => {
    await userEvent.selectOptions(
      await screen.findByRole("combobox", { name: "#general の取り込み方" }),
      "create",
    );
    await userEvent.click(screen.getByRole("button", { name: "1 チャンネルを取り込む" }));
    // 取り込みの進み具合が出て、ZIP はそのまま残り、続けて別のチャンネルを選べる
    await expect(await screen.findByRole("progressbar")).toBeVisible();
    await expect(screen.getByRole("button", { name: "0 チャンネルを取り込む" })).toBeDisabled();
  }),
};

export const StartError: Story = {
  parameters: {
    msw: {
      handlers: [
        list(base),
        http.post("*/api/admin/slack-imports/import-1/start", () =>
          HttpResponse.json({ error: "Channel name 'random' is already taken" }, { status: 409 }),
        ),
      ],
    },
  },
  play: playForImplementation(async () => {
    await userEvent.selectOptions(
      await screen.findByRole("combobox", { name: "#general の取り込み方" }),
      "create",
    );
    await userEvent.click(screen.getByRole("button", { name: "1 チャンネルを取り込む" }));
    await expect(await screen.findByRole("alert")).toHaveTextContent("already taken");
  }),
};

export const Analyzing: Story = {
  parameters: {
    msw: { handlers: [list({ ...base, status: "analyzing", analysis: null, team_id: null })] },
  },
};

export const Importing: Story = {
  parameters: {
    msw: { handlers: [list(run({ processed_messages: 4_210 }), base)] },
  },
  play: playForImplementation(async () => {
    await expect(await screen.findByRole("progressbar")).toHaveAttribute("aria-valuenow", "33");
  }),
};

export const CompletedWithPendingFiles: Story = {
  parameters: {
    msw: {
      handlers: [
        list(
          run({
            status: "completed",
            processed_messages: 12_925,
            total_files: 18,
            pending_files: 16,
            failed_files: 2,
            finished_at: "2026-11-01T09:30:00Z",
          }),
          base,
          { ...base, id: "import-0", status: "cancelled", file_name: "old.zip" },
        ),
      ],
    },
  },
  play: playForImplementation(async () => {
    await expect(
      await screen.findByRole("button", { name: `${base.file_name} の添付ファイルを取得` }),
    ).toBeVisible();
    await expect(screen.getByText(/2 件の添付ファイルを取得できませんでした/)).toBeVisible();
    const history = screen.getByRole("heading", { name: "履歴" }).closest("section")!;
    await expect(within(history).getByText("old.zip")).toBeVisible();
  }),
};

let refetched = "";
export const HistoryWithFailedFiles: Story = {
  beforeEach: () => {
    refetched = "";
  },
  parameters: {
    msw: {
      handlers: [
        list(
          run({ id: "run-2", status: "completed", file_name: "new.zip" }),
          run({ id: "run-1", status: "completed", file_name: "old.zip", failed_files: 3 }),
        ),
        http.post("*/api/admin/slack-imports/:id/fetch-files", ({ params }) => {
          refetched = String(params.id);
          return HttpResponse.json(run({ id: "run-1", status: "fetching_files" }));
        }),
      ],
    },
  },
  play: playForImplementation(async () => {
    await userEvent.click(
      await screen.findByRole("button", { name: "old.zip の添付ファイルを取得" }),
    );
    await waitFor(() => expect(refetched).toBe("run-1"));
  }),
};

export const Failed: Story = {
  parameters: {
    msw: {
      handlers: [
        list({
          ...base,
          status: "failed",
          analysis: null,
          error: "Bad request: channels.json was not found in the export",
        }),
      ],
    },
  },
  play: playForImplementation(async () => {
    await userEvent.click(await screen.findByRole("button", { name: "ZIP を削除" }));
    // 開くアニメーションの途中は不可視と判定されるため、表示されきるまで待つ
    const dialog = await screen.findByRole("alertdialog");
    await waitFor(() => expect(dialog).toBeVisible());
  }),
};

export const FailedImport: Story = {
  parameters: {
    msw: {
      handlers: [
        list(run({ status: "failed", error: "Slack channel C1 was already imported" }), base),
      ],
    },
  },
  play: playForImplementation(async () => {
    await expect(await screen.findByText(/already imported/)).toBeVisible();
    await expect(screen.getByRole("button", { name: "履歴に移す" })).toBeEnabled();
  }),
};

export const Mobile: Story = { ...Mapping, globals: { viewport: { value: "mobile" } } };
