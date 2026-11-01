import { pushStoryControls, pushStoryPresets } from "../../../.storybook/preference-fixtures";
import { preferenceSurfaces } from "../../../.storybook/preference-stories";
import { playForImplementation } from "../../../.storybook/play-for-implementation";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, screen, userEvent, waitFor } from "storybook/test";
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
      <div className={preferenceSurfaces["Features/Preferences/PWA"]}>
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof PwaControls>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {
  play: playForImplementation(async () => {
    await expect(await screen.findByRole("button", { name: /プッシュ通知/ })).toBeDisabled();
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

const push = pushStoryControls;
export const SelectOff: Story = {
  parameters: { a11y: { test: "error" } },
  render: () => <PushNotificationSelect push={pushStoryPresets["Select Off"]} />,
  play: async (context) => {
    await playForImplementation(async () => {
      const select = await screen.findByRole("combobox", { name: "プッシュ通知" });
      await expect(select).toHaveValue("off");
      await userEvent.selectOptions(select, "on");
      await expect(push.toggle).toHaveBeenCalledOnce();
    })(context);
    if (context.globals.implementation === "kotlin")
      await waitFor(() => expect(push.toggle).toHaveBeenCalledOnce());
  },
};
export const SelectOn: Story = {
  parameters: { a11y: { test: "error" } },
  render: () => <PushNotificationSelect push={pushStoryPresets["Select On"]} />,
};
export const SelectBusy: Story = {
  parameters: { a11y: { test: "error" } },
  render: () => <PushNotificationSelect push={pushStoryPresets["Select Busy"]} />,
};
export const SelectBlocked: Story = {
  parameters: { a11y: { test: "error" } },
  render: () => <PushNotificationSelect push={pushStoryPresets["Select Blocked"]} />,
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
