import { playForImplementation } from "../../../.storybook/play-for-implementation";
import { http, HttpResponse } from "msw";
import { expect, fn, userEvent, waitFor, within } from "storybook/test";
import { storyCommands } from "../../../.storybook/app-fixtures";
import type { Meta, StoryObj } from "@storybook/react-vite";

import { StoryShell } from "../../../.storybook/shell";
import { ChannelScreen } from "./ChannelScreen";

const meta = {
  title: "Pages/Channel",
  component: ChannelScreen,
  tags: ["autodocs"],
  parameters: { a11y: { test: "todo" } },
  args: {
    channelId: "channel-general",
  },
  decorators: [
    (Story) => (
      <StoryShell selectedChannelId="channel-general">
        <div className="h-[720px] min-w-0 flex-1 bg-background">
          <Story />
        </div>
      </StoryShell>
    ),
  ],
} satisfies Meta<typeof ChannelScreen>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};

export const PrivateChannel: Story = {
  args: {
    channelId: "channel-secret",
  },
};

export const Archived: Story = {
  args: {
    channelId: "channel-archive",
  },
};

export const NotFound: Story = {
  args: {
    channelId: "channel-missing",
  },
};

export const Mobile: Story = {
  globals: { viewport: { value: "mobile" } },
  decorators: [
    (Story) => (
      <StoryShell selectedChannelId="channel-general" initialPane="chat">
        <div className="h-[720px] min-w-0 flex-1 bg-background">
          <Story />
        </div>
      </StoryShell>
    ),
  ],
};

const commandLookup = fn();
const commandInvocation = fn();
export const CommandCompletion: Story = {
  parameters: {
    a11y: { test: "error" },
    msw: {
      handlers: [
        http.get("*/api/apps/commands", ({ request }) => {
          commandLookup(new URL(request.url).searchParams.get("channel_id"));
          return HttpResponse.json(storyCommands);
        }),
        http.post("*/api/apps/:id/invoke", async ({ params, request }) => {
          commandInvocation(params.id, await request.json());
          return HttpResponse.json({ event_id: "command-event" });
        }),
      ],
    },
  },
  beforeEach: () => {
    commandLookup.mockClear();
    commandInvocation.mockClear();
  },
  play: playForImplementation(
    async ({ canvasElement }) => {
      const canvas = within(canvasElement);
      const input = await canvas.findByRole("textbox");
      await userEvent.type(input, "/ec");
      await userEvent.click(await canvas.findByRole("option", { name: "/echo Meeting Notes" }));
      // Composer restores the caret on the next animation frame after completion.
      await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
      await userEvent.type(input, "hello");
      await userEvent.click(canvas.getByRole("button", { name: "送信" }));
      await waitFor(() =>
        expect(commandInvocation).toHaveBeenCalledWith("notes", {
          kind: "command",
          name: "echo",
          channel_id: "channel-general",
          values: { text: "hello" },
        }),
      );
      await expect(commandLookup).toHaveBeenCalledWith("channel-general");
      await expect(input).toHaveValue("");
    },
    async (context) => {
      if (context.globals.implementation === "kotlin") {
        await waitFor(() =>
          expect(commandInvocation).toHaveBeenCalledWith("notes", {
            kind: "command",
            name: "echo",
            channel_id: "channel-general",
            values: { text: "hello" },
          }),
        );
        await expect(commandLookup).toHaveBeenCalledWith("channel-general");
      }
    },
  ),
};

const storyNote = {
  id: "note-story",
  channel_id: "channel-general",
  title: "チームノート",
  content:
    "# 今日の予定\n\n- [ ] リリースを確認\n- [x] レビューを依頼\n\n共有事項をここに記録します。",
  version: 1,
  created_user_id: "user-alice",
  created_at: "2026-01-02T09:00:00.000Z",
  updated_at: "2026-01-02T09:00:00.000Z",
};
const noteHandlers = [
  http.get("*/api/channels/:channelId/notes", () => {
    const { content: _content, ...summary } = storyNote;
    return HttpResponse.json({ data: [summary], txid: null });
  }),
  http.get("*/api/channels/:channelId/notes/note-story", () =>
    HttpResponse.json({ data: storyNote, txid: null }),
  ),
];

export const Notes: Story = {
  args: { noteId: "note-story" },
  parameters: { a11y: { test: "error" }, msw: { handlers: noteHandlers } },
};
export const NotesEmpty: Story = {
  parameters: { a11y: { test: "error" } },
};
export const NotesLoadError: Story = {
  args: { noteId: "note-story" },
  parameters: {
    a11y: { test: "error" },
    msw: {
      handlers: [
        http.get("*/api/channels/:channelId/notes", () =>
          HttpResponse.json({ error: "Unavailable" }, { status: 500 }),
        ),
      ],
    },
  },
};
export const NotesReadOnly: Story = {
  args: { channelId: "channel-archive", noteId: "note-story" },
  parameters: { a11y: { test: "error" }, msw: { handlers: noteHandlers } },
};
export const NotesMobile: Story = {
  ...Notes,
  globals: { viewport: { value: "mobile" } },
};
export const NotesDeleted: Story = {
  parameters: {
    a11y: { test: "error" },
    msw: {
      handlers: [
        ...noteHandlers,
        http.get("*/api/channels/:channelId/notes/deleted", () =>
          HttpResponse.json({ data: [storyNote], txid: null }),
        ),
      ],
    },
  },
  play: playForImplementation(async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(await canvas.findByRole("button", { name: "削除済み" }));
    await expect(await canvas.findByRole("button", { name: /復元.*チームノート/ })).toBeVisible();
  }),
};
export const NotesConflict: Story = {
  ...Notes,
  parameters: {
    a11y: { test: "error" },
    msw: {
      handlers: [
        ...noteHandlers,
        http.patch("*/api/channels/:channelId/notes/:noteId", () =>
          HttpResponse.json({ error: "Conflict" }, { status: 409 }),
        ),
      ],
    },
  },
  play: playForImplementation(async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(await canvas.findByRole("button", { name: "編集" }));
    await userEvent.type(canvas.getByRole("textbox", { name: "本文" }), "追記");
    await userEvent.click(canvas.getByRole("button", { name: "ノートを保存" }));
    await expect(await canvas.findByText(/最新版の内容/)).toBeVisible();
  }),
};
