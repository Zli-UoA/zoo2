import { storySurfaces } from "../../../.storybook/story-surfaces";
import { playForImplementation } from "../../../.storybook/play-for-implementation";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { bubbleChatHandlers } from "../../../.storybook/bubble-fixtures";
import { expect, screen, userEvent, waitFor, within } from "storybook/test";

import { StoryShell } from "../../../.storybook/shell";
import { BubbleChatScreen } from "./BubbleChatScreen";

const meta = {
  title: "Pages/BubbleChat",
  component: BubbleChatScreen,
  tags: ["autodocs"],
  parameters: {
    a11y: { test: "error" },
    router: { initialEntries: ["/bubble"] },
  },
  decorators: [
    (Story) => (
      <StoryShell selectedChannelId={null} initialPane="chat">
        <div className={storySurfaces["Pages/BubbleChat"]}>
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
      handlers: bubbleChatHandlers,
    },
  },
  play: playForImplementation(async () => {
    await expect(
      await screen.findByText("今日のミーティングは何時からでしたっけ？"),
    ).toBeInTheDocument();
    await expect(await screen.findByText("15時からだった気がします！")).toBeInTheDocument();

    const input = screen.getByRole("textbox", { name: "リアルタイムメッセージ入力" });
    await userEvent.type(input, "ありがとう！準備してきます");
    const list = screen.getByRole("list");
    await expect(await within(list).findByText("ありがとう！準備してきます")).toBeInTheDocument();

    await userEvent.keyboard("{Control>}{Enter}{/Control}");
    await expect(input).toHaveValue("");
    // 入力欄は即座にクリアされるが、吹き出しは破裂アニメーションのため少しの間表示される
    await expect(await within(list).findByText("ありがとう！準備してきます")).toBeInTheDocument();
  }),
};

export const Empty: Story = {
  play: playForImplementation(async () => {
    await expect(
      await screen.findByText("現在入力中のユーザーはいません", { selector: "p" }),
    ).toBeInTheDocument();
  }),
};

export const Mobile: Story = {
  globals: { viewport: { value: "mobile" } },
  parameters: {
    msw: {
      handlers: bubbleChatHandlers,
    },
  },
};

export const Participants: Story = {
  parameters: { msw: { handlers: bubbleChatHandlers } },
  play: playForImplementation(async () => {
    await screen.findByText("今日のミーティングは何時からでしたっけ？");
    await userEvent.click(screen.getByRole("button", { name: /^参加者 / }));
    const panel = await screen.findByRole("complementary", { name: "参加者" });
    await expect(within(panel).getByText("@bob")).toBeVisible();
  }),
};
export const MobileParticipants: Story = {
  globals: { viewport: { value: "mobile" } },
  parameters: { msw: { handlers: bubbleChatHandlers } },
  play: playForImplementation(async () => {
    await screen.findByText("今日のミーティングは何時からでしたっけ？");
    await userEvent.click(screen.getByRole("button", { name: /^参加者 / }));
    const dialog = await screen.findByRole("dialog");
    await waitFor(() => expect(within(dialog).getByText("@bob")).toBeVisible());
  }),
};
