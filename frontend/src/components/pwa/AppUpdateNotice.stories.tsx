import type { Meta, StoryObj } from "@storybook/react-vite";
import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { expect, fn, userEvent, within } from "storybook/test";

import { Toaster } from "../ui/sonner";
import { showAppUpdateToast } from "./AppUpdateNotice";

const reload = fn();

/** デプロイされた画面が新しくなったときの知らせ。 */
function UpdateToast() {
  const { t } = useTranslation();
  useEffect(() => {
    showAppUpdateToast(t, reload);
    return () => {
      toast.dismiss();
    };
  }, [t]);
  return <Toaster />;
}

const meta = {
  title: "Features/Status/AppUpdate",
  component: UpdateToast,
  parameters: { a11y: { test: "error" } },
  globals: { locale: "ja" },
  decorators: [
    (Story) => (
      <div className="min-h-48 bg-background">
        <Story />
      </div>
    ),
  ],
  beforeEach: () => {
    reload.mockClear();
    toast.dismiss();
  },
} satisfies Meta<typeof UpdateToast>;
export default meta;
type Story = StoryObj<typeof meta>;

export const Available: Story = {
  play: async ({ canvasElement }) => {
    const body = within(canvasElement.ownerDocument.body);
    await expect(await body.findByText("新しい版の画面があります")).toBeVisible();
    await userEvent.click(await body.findByRole("button", { name: "再読み込み" }));
    await expect(reload).toHaveBeenCalledTimes(1);
  },
};

export const Mobile: Story = { globals: { viewport: { value: "mobile" } } };
