import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, fireEvent, fn, userEvent, within } from "storybook/test";

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
    finishingLabel: "保存しています…",
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
    const button = await within(canvasElement).findByRole("button", {
      name: "report.pdfをダウンロード",
    });
    await userEvent.click(button);
    await expect(args.onDownload).toHaveBeenCalledTimes(1);
    // 中止のダブルクリックの 2 回目が、開始ボタンに戻った後に届いても開始しない。
    await fireEvent.click(button, { detail: 2 });
    await expect(args.onDownload).toHaveBeenCalledTimes(1);
    // ライブリージョンは最初から DOM に置き、保全中は空にしておく。
    await expect(within(canvasElement).getByRole("status")).toHaveTextContent("");
  },
};

/** 保存ダイアログを出している間。進み具合が分からないので、円を回す。 */
export const Preparing: Story = {
  args: { state: { status: "preparing" } },
  play: async ({ canvasElement }) => {
    const button = await within(canvasElement).findByRole("button", {
      name: "report.pdfのダウンロードを中止",
    });
    await expect(button).toBeEnabled();
  },
};

export const Downloading: Story = {
  args: { state: { status: "downloading", fraction: 0.42 } },
  play: async ({ canvasElement, args }) => {
    const button = await within(canvasElement).findByRole("button", {
      name: "report.pdfのダウンロードを中止 42%",
    });
    await userEvent.click(button);
    await expect(args.onCancel).toHaveBeenCalledTimes(1);
    // ダブルクリックの 2 回目では、もう一度中止しない。
    await fireEvent.click(button, { detail: 2 });
    await expect(args.onCancel).toHaveBeenCalledTimes(1);
    // 進捗は 10% 刻みで読み上げる。フォーカスが別の添付に移っても伝わる。
    await expect(within(canvasElement).getByRole("status")).toHaveTextContent("ダウンロード中 40%");
  },
};

/** 大きさが分からない応答。円は回したままにする。 */
export const UnknownSize: Story = {
  args: { state: { status: "downloading", fraction: null } },
};

/** 本文を書き終え、名前に置き換えている間。円は完成した状態で待つ。中止はできない。 */
export const Finishing: Story = {
  args: { state: { status: "finishing" } },
  play: async ({ canvasElement }) => {
    const button = await within(canvasElement).findByRole("button", { name: "保存しています…" });
    await expect(button).toBeDisabled();
    await expect(within(canvasElement).getByRole("status")).toHaveTextContent("保存しています…");
  },
};

/** 名前付きのボタンでも、確認中は「中止」ではなく保存中と出す。 */
export const FinishingWithLabel: Story = {
  args: { state: { status: "finishing" }, showLabel: true },
  play: async ({ canvasElement }) => {
    const button = await within(canvasElement).findByRole("button", { name: "保存しています…" });
    await expect(button).toBeDisabled();
    await expect(button).toHaveTextContent("保存しています…");
  },
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
