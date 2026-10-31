import { playForImplementation } from "../../../.storybook/play-for-implementation";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, fn, screen, userEvent } from "storybook/test";
import { useEffect, type ReactNode } from "react";

import { PwaProvider, PwaControls, PushNotificationSelect } from "./PwaControls";

function InstallPrompt({ children }: { children: ReactNode }) {
  useEffect(() => {
    const event = new Event("beforeinstallprompt", { cancelable: true });
    Object.assign(event, {
      prompt: () => Promise.resolve(),
      userChoice: Promise.resolve({ outcome: "accepted" }),
    });
    window.dispatchEvent(event);
  }, []);
  return <>{children}</>;
}

const meta = {
  title: "Features/Preferences/PWA",
  component: PwaControls,
  tags: ["autodocs"],
  parameters: { a11y: { test: "todo" } },
  decorators: [
    (Story) => (
      <div className="flex min-h-72 items-center justify-center gap-2 bg-background p-6">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof PwaControls>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {
  play: playForImplementation(async () => {
    await expect(screen.getByRole("button", { name: /プッシュ通知/ })).toBeDisabled();
  }),
};

export const InstallAvailable: Story = {
  decorators: [
    (Story) => (
      <InstallPrompt>
        <PwaProvider>
          <Story />
        </PwaProvider>
      </InstallPrompt>
    ),
  ],
  play: playForImplementation(async () => {
    await expect(await screen.findByRole("button", { name: "Zooをインストール" })).toBeEnabled();
  }),
};

const push = {
  label: "プッシュ通知を有効にする",
  enabled: false,
  busy: false,
  supported: true,
  publicKey: "storybook-key",
  blocked: false,
  toggle: fn(async () => {}),
};
export const SelectOff: Story = {
  parameters: { a11y: { test: "error" } },
  render: () => <PushNotificationSelect push={push} />,
  play: playForImplementation(async () => {
    const select = screen.getByRole("combobox", { name: "プッシュ通知" });
    await expect(select).toHaveValue("off");
    await userEvent.selectOptions(select, "on");
    await expect(push.toggle).toHaveBeenCalledOnce();
  }),
};
export const SelectOn: Story = {
  parameters: { a11y: { test: "error" } },
  render: () => <PushNotificationSelect push={{ ...push, enabled: true }} />,
};
export const SelectBusy: Story = {
  parameters: { a11y: { test: "error" } },
  render: () => <PushNotificationSelect push={{ ...push, busy: true }} />,
};
export const SelectBlocked: Story = {
  parameters: { a11y: { test: "error" } },
  render: () => (
    <PushNotificationSelect
      push={{ ...push, blocked: true, label: "ブラウザーの設定で通知を許可してください" }}
    />
  ),
};
export const SelectUnavailable: Story = {
  args: { presentation: "select" },
  parameters: { a11y: { test: "error" } },
};
export const SelectInstallAvailable: Story = {
  ...InstallAvailable,
  args: { presentation: "select" },
  parameters: { a11y: { test: "error" } },
};
