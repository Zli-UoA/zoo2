import type { Meta, StoryObj } from "@storybook/react-vite";

import { Textarea } from "./textarea";

const meta = {
  title: "Foundations/Textarea",
  component: Textarea,
  tags: ["autodocs"],
  parameters: { a11y: { test: "error" } },
  argTypes: {
    disabled: { control: "boolean" },
    placeholder: { control: "text" },
  },
  args: {
    placeholder: "メッセージを入力",
    "aria-label": "メッセージ",
  },
  decorators: [
    (Story) => (
      <div className="max-w-xl bg-background p-6">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof Textarea>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};

export const WithValue: Story = {
  args: {
    defaultValue: "今日のリリースは 18:00 です。",
  },
};

export const Disabled: Story = {
  args: {
    disabled: true,
    defaultValue: "編集できません",
  },
};

export const Invalid: Story = {
  args: {
    "aria-invalid": true,
    defaultValue: "入力内容を確認してください",
  },
};

export const LongText: Story = {
  args: {
    defaultValue: Array.from(
      { length: 6 },
      (_, index) =>
        `${index + 1} 行目: リリース手順の確認項目です。担当者と期限を確認してから次へ進んでください。`,
    ).join("\n"),
  },
};
