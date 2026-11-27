import { http, HttpResponse } from "msw";
import { expect, userEvent, within } from "storybook/test";
import type { Meta, StoryObj } from "@storybook/react-vite";

import type { ChannelNote, ChannelNoteSummary } from "../../../lib/api/channelNotes";
import { ChannelNotesScreen } from "./ChannelNotesScreen";

const CHANNEL = "00000000-0000-4000-8000-000000000001";
const TIMESTAMP = "2026-11-28T01:02:03.000";

function summary(
  id: string,
  title: string,
  position: number,
  orderToken = "story-token",
): ChannelNoteSummary {
  return {
    id,
    channel_id: CHANNEL,
    title,
    position,
    version: 1,
    created_user_id: "u-alice",
    created_at: TIMESTAMP,
    updated_at: TIMESTAMP,
    order_token: orderToken,
  };
}

function note(id: string, title: string, content: string): ChannelNote {
  return { ...summary(id, title, 0), content };
}

const summaries = [
  summary("note-runbook", "Runbook", 0),
  summary("note-release", "リリース手順", 1),
  summary("note-oncall", "On-call checklist", 2),
];

const meta = {
  title: "Pages/Channel/Notes",
  component: ChannelNotesScreen,
  tags: ["autodocs"],
  parameters: {
    a11y: { test: "error" },
    msw: {
      handlers: [
        http.get("*/api/channels/:channelId/notes", ({ request }) =>
          HttpResponse.json({
            data: new URL(request.url).searchParams.get("after") ? [] : summaries,
            txid: null,
          }),
        ),
        http.get("*/api/channels/:channelId/notes/:noteId", ({ params }) =>
          HttpResponse.json({
            data: note(
              String(params.noteId),
              summaries.find((item) => item.id === params.noteId)?.title ?? "Note",
              "本文です。\n\n- 手順 1\n- 手順 2\n\n$[clickable.ev=story 表示専用]",
            ),
            txid: null,
          }),
        ),
      ],
    },
  },
  args: {
    channelId: CHANNEL,
    currentUserId: "u-alice",
    canRead: true,
    canWrite: true,
    isMember: true,
    isArchived: false,
  },
  decorators: [
    (Story) => (
      <div style={{ display: "flex", height: 720, width: "100%" }}>
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof ChannelNotesScreen>;

export default meta;
type Story = StoryObj<typeof meta>;

export const List: Story = {};

export const Detail: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(await canvas.findByRole("button", { name: "Runbook" }));
    await expect(await canvas.findByText(/手順 1/)).toBeVisible();
    await expect(canvas.queryByRole("button", { name: "表示専用" })).toBeNull();
  },
};

export const Mobile: Story = {
  globals: { viewport: { value: "mobile" } },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(await canvas.findByRole("button", { name: "Runbook" }));
    await expect(await canvas.findByRole("button", { name: "ノート一覧に戻る" })).toBeVisible();
    await userEvent.click(canvas.getByRole("button", { name: "ノート一覧に戻る" }));
    await expect(await canvas.findByRole("button", { name: "Runbook" })).toBeVisible();
  },
};
