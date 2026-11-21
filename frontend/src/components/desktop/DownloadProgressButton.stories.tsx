import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, fn, userEvent, within } from "storybook/test";

import { DownloadProgressButton } from "./DownloadProgressButton";

const meta = {
  title: "Components/DownloadProgressButton",
  component: DownloadProgressButton,
  parameters: { a11y: { test: "error" } },
  args: {
    state: undefined,
    pending: false,
    showLabel: false,
    downloadLabel: "report.pdfをダウンロード",
    downloadShortLabel: "ダウンロード",
    cancelLabel: "report.pdfのダウンロードを中止",
    cancelShortLabel: "中止",
    progressLabel: "ダウンロード中",
    onDownload: fn(),
    onCancel: fn(),
  },
  decorators: [
    (Story) => (
      <div className="flex min-h-24 items-center justify-center bg-background p-6">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof DownloadProgressButton>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Idle: Story = {
  play: async ({ canvasElement, args }) => {
    await userEvent.click(
      await within(canvasElement).findByRole("button", { name: "report.pdfをダウンロード" }),
    );
    await expect(args.onDownload).toHaveBeenCalledTimes(1);
  },
};

/** 保存ダイアログを出している間。進み具合が分からないので、円を回す。 */
export const Preparing: Story = {
  args: { state: { status: "preparing" } },
  play: async ({ canvasElement }) => {
    const progress = await within(canvasElement).findByRole("progressbar", {
      name: "ダウンロード中",
    });
    await expect(progress).not.toHaveAttribute("aria-valuenow");
  },
};

export const Downloading: Story = {
  args: { state: { status: "downloading", fraction: 0.42 } },
  play: async ({ canvasElement, args }) => {
    const progress = await within(canvasElement).findByRole("progressbar", {
      name: "ダウンロード中",
    });
    await expect(progress).toHaveAttribute("aria-valuenow", "42");
    await userEvent.click(
      await within(canvasElement).findByRole("button", { name: "report.pdfのダウンロードを中止" }),
    );
    await expect(args.onCancel).toHaveBeenCalledTimes(1);
  },
};

/** 大きさが分からない応答。円は回したままにする。 */
export const UnknownSize: Story = {
  args: { state: { status: "downloading", fraction: null } },
};

/** 本文を書き終え、名前に置き換えている間。円は完成した状態で待つ。 */
export const Finishing: Story = {
  args: { state: { status: "finishing" } },
};

export const Complete: Story = {
  args: { state: { status: "downloading", fraction: 1 } },
};

export const WithLabel: Story = {
  args: { state: { status: "downloading", fraction: 0.7 }, showLabel: true },
};

/** デスクトップ版ではないときの、ブラウザーの保存の準備中。 */
export const Pending: Story = {
  args: { pending: true },
};
