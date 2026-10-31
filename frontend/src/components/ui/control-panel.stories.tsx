import type { Meta, StoryObj } from "@storybook/react-vite";
import { SettingsSection, SettingRow } from "./control-panel";
import { Button } from "./button";
const meta = {
  title: "Components/ControlPanel",
  component: SettingsSection,
  parameters: { a11y: { test: "error" } },
  decorators: [
    (Story) => (
      <div className="mx-auto max-w-xl p-4">
        <Story />
      </div>
    ),
  ],
  args: {
    title: "メンバー",
    description: "メンバーと権限を管理します。",
    children: (
      <SettingRow label="Alice" description="管理者">
        <Button variant="outline">権限を変更</Button>
      </SettingRow>
    ),
  },
} satisfies Meta<typeof SettingsSection>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Default: Story = {};
export const LongDescription: Story = {
  args: {
    children: (
      <SettingRow
        label="長い表示名のメンバー"
        description={"名前や説明が長くても操作ボタンと重ならずに読めます。".repeat(8)}
      >
        <Button variant="outline">変更</Button>
      </SettingRow>
    ),
  },
};
export const Mobile: Story = { ...LongDescription, globals: { viewport: { value: "mobile" } } };
