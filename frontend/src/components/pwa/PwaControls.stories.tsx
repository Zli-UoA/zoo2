import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, screen } from "storybook/test";
import { useEffect, type ReactNode } from "react";

import { PwaControls } from "./PwaControls";

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
  title: "Layout/PwaControls",
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
  play: async () => {
    await expect(screen.getByRole("button", { name: /プッシュ通知/ })).toBeDisabled();
  },
};

export const InstallAvailable: Story = {
  decorators: [
    (Story) => (
      <InstallPrompt>
        <Story />
      </InstallPrompt>
    ),
  ],
  play: async () => {
    await expect(await screen.findByRole("button", { name: "Zooをインストール" })).toBeEnabled();
  },
};
