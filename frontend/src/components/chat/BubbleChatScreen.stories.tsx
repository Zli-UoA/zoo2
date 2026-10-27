import type { Meta, StoryObj } from "@storybook/react-vite";
import { http, HttpResponse } from "msw";
import { expect, screen, userEvent } from "storybook/test";

import { StoryShell } from "../../../.storybook/shell";
import { BubbleChatScreen } from "./BubbleChatScreen";

const activeBubblesHandler = http.get("*/api/bubble-chat", () =>
  HttpResponse.json([
    {
      user_id: "user-bob",
      user_name: "bob",
      display_name: "Bob",
      content: "今日のミーティングは何時からでしたっけ？",
    },
    {
      user_id: "user-carol",
      user_name: "carol",
      display_name: "Carol",
      content: "15時からだった気がします！",
    },
  ]),
);

const meta = {
  title: "Chat/BubbleChatScreen",
  component: BubbleChatScreen,
  tags: ["autodocs"],
  parameters: {
    a11y: { test: "error" },
    router: { initialEntries: ["/bubble-chat"] },
  },
  decorators: [
    (Story) => (
      <StoryShell selectedChannelId={null} initialPane="chat">
        <div className="h-[720px] overflow-hidden bg-background">
          <Story />
        </div>
      </StoryShell>
    ),
  ],
} satisfies Meta<typeof BubbleChatScreen>;

export default meta;
type Story = StoryObj<typeof meta>;

export const ActiveConversation: Story = {
  parameters: {
    msw: {
      handlers: [activeBubblesHandler],
    },
  },
  play: async () => {
    await expect(
      await screen.findByText("今日のミーティングは何時からでしたっけ？"),
    ).toBeInTheDocument();
    await expect(await screen.findByText("15時からだった気がします！")).toBeInTheDocument();

    const input = screen.getByRole("textbox", { name: "リアルタイムメッセージ入力" });
    await userEvent.type(input, "ありがとう！準備してきます");
    await expect(await screen.findByText("ありがとう！準備してきます")).toBeInTheDocument();
  },
};

export const Empty: Story = {
  play: async () => {
    await expect(await screen.findByText("現在入力中のユーザーはいません")).toBeInTheDocument();
  },
};

export const Mobile: Story = {
  globals: { viewport: { value: "mobile" } },
  parameters: {
    msw: {
      handlers: [activeBubblesHandler],
    },
  },
};
