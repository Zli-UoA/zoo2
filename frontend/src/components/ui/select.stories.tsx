import type { Meta, StoryObj } from "@storybook/react-vite";
import { Select } from "./select";
const meta = {
  title: "Components/Select",
  component: Select,
  parameters: { layout: "centered", a11y: { test: "error" } },
  args: {
    "aria-label": "テーマ",
    defaultValue: "system",
    children: (
      <>
        <option value="light">ライト</option>
        <option value="dark">ダーク</option>
        <option value="system">システム</option>
      </>
    ),
  },
} satisfies Meta<typeof Select>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Default: Story = {};
export const Disabled: Story = { args: { disabled: true } };
export const LongLabel: Story = {
  args: {
    defaultValue: "long",
    children: <option value="long">長い選択肢の名前でも幅を広げません</option>,
  },
};
