import type { Meta, StoryObj } from "@storybook/react-vite";
import { delay, http, HttpResponse } from "msw";
import { expect, screen, userEvent, within } from "storybook/test";
import type { SlackImport } from "../../lib/api/slackImport";
import { SlackImportPanel } from "./SlackImportPanel";

const base: SlackImport = {
  id: "import-1",
  status: "awaiting_mapping",
  file_name: "Zli Slack export Jan 1 2024 - Nov 1 2026.zip",
  file_size: 12_345_678,
  team_id: "T1",
  analysis: {
    team_id: "T1",
    user_count: 42,
    channels: [
      {
        slack_channel_id: "C1",
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
        name: "very-long-private-channel-name-for-the-infrastructure-working-group",
        is_private: true,
        is_archived: false,
        members: 5,
        message_count: 31,
        suggested_name: "very-long-private-channel-name-for-the-infrastructure-working-group",
        name_taken: false,
        existing_channel: null,
      },
    ],
  },
  total_messages: 12_925,
  processed_messages: 0,
  total_files: 0,
  processed_files: 0,
  pending_files: 0,
  error: null,
  created_at: "2026-11-01T09:00:00Z",
  finished_at: null,
};

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
  play: async () => {
    await expect(
      await screen.findByRole("button", { name: "ZIP を選んでアップロード" }),
    ).toBeEnabled();
  },
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
  play: async () => {
    await expect(await screen.findByRole("alert")).toHaveTextContent("管理者だけ");
  },
};

export const Mapping: Story = {
  parameters: { msw: { handlers: [list(base)] } },
  play: async () => {
    const name = await screen.findByRole("textbox", { name: "#general の新しいチャンネル名" });
    await expect(name).toHaveValue("slack-general");
    await userEvent.selectOptions(
      screen.getByRole("combobox", { name: "#general の取り込み方" }),
      "skip",
    );
    await expect(screen.getByRole("button", { name: "2 チャンネルを取り込む" })).toBeEnabled();
  },
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
  play: async () => {
    await userEvent.click(await screen.findByRole("button", { name: "3 チャンネルを取り込む" }));
    await expect(await screen.findByRole("alert")).toHaveTextContent("already taken");
  },
};

export const Analyzing: Story = {
  parameters: {
    msw: { handlers: [list({ ...base, status: "analyzing", analysis: null, team_id: null })] },
  },
};

export const Importing: Story = {
  parameters: {
    msw: { handlers: [list({ ...base, status: "importing", processed_messages: 4_210 })] },
  },
  play: async () => {
    await expect(await screen.findByRole("progressbar")).toHaveAttribute("aria-valuenow", "33");
  },
};

export const CompletedWithPendingFiles: Story = {
  parameters: {
    msw: {
      handlers: [
        list(
          {
            ...base,
            status: "completed",
            processed_messages: 12_925,
            total_files: 18,
            pending_files: 18,
            finished_at: "2026-11-01T09:30:00Z",
          },
          { ...base, id: "import-0", status: "cancelled", file_name: "old.zip" },
        ),
      ],
    },
  },
  play: async () => {
    await expect(await screen.findByRole("button", { name: "添付ファイルを取得" })).toBeVisible();
    const history = screen.getByRole("heading", { name: "履歴" }).closest("section")!;
    await expect(within(history).getByText("old.zip")).toBeVisible();
  },
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
  play: async () => {
    await userEvent.click(await screen.findByRole("button", { name: "取り消す" }));
    await expect(await screen.findByRole("alertdialog")).toBeVisible();
  },
};

export const Mobile: Story = { ...Mapping, globals: { viewport: { value: "mobile" } } };
