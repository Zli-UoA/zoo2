import { playForImplementation } from "../../../.storybook/play-for-implementation";
import { http, HttpResponse } from "msw";
import { expect, fn, userEvent, waitFor, within } from "storybook/test";
import { storyCommands } from "../../../.storybook/app-fixtures";
import type { Meta, StoryObj } from "@storybook/react-vite";

import { StoryShell } from "../../../.storybook/shell";
import { storyCollections } from "../../../.storybook/fixtures";
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
  play: playForImplementation(async ({ canvasElement }) => {
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
  }),
};

export const Nsfw: Story = {
  parameters: {
    a11y: { test: "error" },
    collections: {
      ...storyCollections,
      channels: storyCollections.channels.map((channel) => ({
        ...channel,
        is_nsfw: channel.id === "channel-general",
      })),
    },
  },
  play: playForImplementation(async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(
      await canvas.findByText("このチャンネルにはセンシティブな内容が含まれる場合があります。"),
    ).toBeVisible();
    await expect(canvas.queryByRole("textbox", { name: "メッセージ" })).not.toBeInTheDocument();
    await userEvent.click(canvas.getByRole("button", { name: "チャンネルを表示" }));
    await waitFor(() =>
      expect(
        canvas.queryByText("このチャンネルにはセンシティブな内容が含まれる場合があります。"),
      ).not.toBeInTheDocument(),
    );
  }),
};
