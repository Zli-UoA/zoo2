import type { Meta, StoryObj } from "@storybook/react-vite";

import { Skeleton } from "./skeleton";

function EmptyState() {
  return (
    <div className="flex h-72 flex-col items-center justify-center gap-2 bg-background p-6 text-center">
      <p className="text-sm font-medium">まだメッセージがありません</p>
      <p className="text-xs text-muted-foreground">最初のメッセージを送ってみましょう。</p>
    </div>
  );
}

function ErrorState() {
  return (
    <div className="flex h-72 flex-col items-center justify-center gap-2 bg-background p-6 text-center">
      <p role="alert" className="text-sm font-medium text-destructive">
        メッセージの取得に失敗しました
      </p>
      <p className="text-xs text-muted-foreground">接続を確認して、もう一度お試しください。</p>
    </div>
  );
}

function LoadingState() {
  return (
    <div className="flex h-72 flex-col gap-4 bg-background p-6" aria-busy="true" aria-live="polite">
      <span className="sr-only">読み込み中</span>
      {[0, 1, 2, 3].map((row) => (
        <div key={row} className="flex gap-3">
          <Skeleton className="size-9 shrink-0 rounded-full" />
          <div className="flex flex-1 flex-col gap-2">
            <Skeleton className="h-3 w-28" />
            <Skeleton className="h-4 w-full max-w-md" />
          </div>
        </div>
      ))}
    </div>
  );
}

const meta = {
  title: "Foundations/States",
  tags: ["autodocs"],
  parameters: { a11y: { test: "error" } },
  component: EmptyState,
} satisfies Meta<typeof EmptyState>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Empty: Story = {};

export const Error: Story = {
  render: () => <ErrorState />,
};

export const Loading: Story = {
  render: () => <LoadingState />,
};
