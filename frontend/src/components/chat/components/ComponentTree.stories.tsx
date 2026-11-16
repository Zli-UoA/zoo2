import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, within } from "storybook/test";
import { storySurfaces } from "../../../../.storybook/story-surfaces";
import all from "../../../../../crates/zoo-components/fixtures/valid/message-all-display.json";
import interactive from "../../../../../crates/zoo-components/fixtures/valid/message-interactive.json";
import modal from "../../../../../crates/zoo-components/fixtures/valid/modal-inputs.json";
import { ComponentTree } from "./ComponentTree";
import { parseComponents } from "./parse";

const meta = {
  title: "Features/Chat/MessageComponents",
  component: ComponentTree,
  tags: ["autodocs"],
  parameters: { a11y: { test: "error" } },
  args: { document: parseComponents(all.document)! },
  decorators: [
    (Story) => (
      <div className={storySurfaces["Features/Chat/MessageComponents"]}>
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof ComponentTree>;

export default meta;
type Story = StoryObj<typeof meta>;

/** 表示だけの部品をすべて並べたもの（container・section・ギャラリー・表・グラフ・カードなど）。 */
export const AllDisplay: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByText("Section text")).toBeInTheDocument();
    await expect(canvas.getByRole("table")).toBeInTheDocument();
    await expect(canvas.getByRole("link", { name: "Open" })).toHaveAttribute(
      "rel",
      "noopener noreferrer",
    );
  },
};

/** アプリのメッセージの操作部品。アプリがないので押せない状態（Webhook と同じ見た目）。 */
export const InteractiveWithoutApp: Story = {
  args: { document: parseComponents(interactive.document)! },
  play: async ({ canvasElement }) => {
    await expect(within(canvasElement).getByRole("button", { name: "Approve" })).toBeDisabled();
  },
};

/** アプリのメッセージの操作部品（押せる）。 */
export const Interactive: Story = {
  args: {
    document: parseComponents(interactive.document)!,
    appId: "app",
    messageId: "m1",
    channelId: "c1",
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByRole("button", { name: "Approve" })).toBeEnabled();
    await expect(canvas.getByRole("button", { name: "Off" })).toBeDisabled();
  },
};

/** モーダルの入力欄（Label と text input・radio・checkbox・file upload など）。 */
export const ModalInputs: Story = {
  args: {
    document: parseComponents(modal.document)!,
    appId: "app",
    surfaceId: "s1",
    channelId: "c1",
    submit: true,
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByLabelText("Name")).toBeRequired();
    await expect(canvas.getByRole("button", { name: /送信|Submit/ })).toBeInTheDocument();
  },
};

/** 狭い配置先（スレッドやサイドパネル）。 */
export const Narrow: Story = {
  decorators: [
    (Story) => (
      <div className="w-72">
        <Story />
      </div>
    ),
  ],
};
