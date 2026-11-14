import { preferenceSurfaces } from "../../../.storybook/preference-surfaces";
import { playForImplementation } from "../../../.storybook/play-for-implementation";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, screen, userEvent } from "storybook/test";
import { BellIcon } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";

import { DESKTOP_PROTOCOL_VERSION, detectDesktop, resetDesktopForTest } from "../../lib/desktop";
import { SettingRow } from "../ui/control-panel";
import { DesktopNotificationSelect } from "./DesktopNotificationSelect";

/** デスクトップ版 (Tauri) の中にいるように見せる。`supported` が false なら古い版のアプリ。 */
function FakeDesktop({
  enabled,
  supported,
  children,
}: {
  enabled: boolean;
  supported: boolean;
  children: ReactNode;
}) {
  const [ready, setReady] = useState(false);
  useEffect(() => {
    let current = enabled;
    resetDesktopForTest();
    Object.assign(window, {
      __TAURI_INTERNALS__: {
        invoke: async (cmd: string, args?: { settings?: { enabled: boolean } }) => {
          if (cmd === "zoo_session")
            return {
              protocol: DESKTOP_PROTOCOL_VERSION,
              ui: "remote",
              api_base_url: "https://zli.chat",
              state: { state: "ready" },
            };
          if (!supported) throw new Error(`command ${cmd} not found`);
          if (cmd === "zoo_notification_settings") return { enabled: current };
          if (cmd === "zoo_set_notifications") {
            current = args?.settings?.enabled ?? current;
            return null;
          }
          return null;
        },
        transformCallback: () => 1,
      },
    });
    void detectDesktop().then(() => setReady(true));
    return () => {
      delete (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
      resetDesktopForTest();
    };
  }, [enabled, supported]);
  return ready ? <>{children}</> : null;
}

const meta = {
  title: "Features/Preferences/DesktopNotifications",
  component: DesktopNotificationSelect,
  parameters: { a11y: { test: "error" } },
  // 行の名前 (「通知」) は日本語で書いているので、ツールバーのロケールによらず日本語で出す。
  globals: { locale: "ja" },
  decorators: [
    (Story) => (
      <div className={preferenceSurfaces["Features/Preferences/DesktopNotifications"]}>
        {/* 設定画面 (ZooSettings) と同じ行に置く。 */}
        <div className="w-full rounded-lg border">
          <SettingRow
            icon={<BellIcon />}
            label="通知"
            description="メンションや DM を、アプリが OS の通知で知らせます。OS の設定で zoo2 の通知を止めている場合は、そちらもオンにしてください。"
          >
            <Story />
          </SettingRow>
        </div>
      </div>
    ),
  ],
} satisfies Meta<typeof DesktopNotificationSelect>;

export default meta;
type Story = StoryObj<typeof meta>;

export const On: Story = {
  decorators: [
    (Story) => (
      <FakeDesktop enabled supported>
        <Story />
      </FakeDesktop>
    ),
  ],
  play: playForImplementation(async () => {
    await expect(await screen.findByRole("combobox", { name: "通知" })).toHaveValue("on");
  }),
};

export const TurnOff: Story = {
  decorators: [
    (Story) => (
      <FakeDesktop enabled supported>
        <Story />
      </FakeDesktop>
    ),
  ],
  play: playForImplementation(async () => {
    const select = await screen.findByRole("combobox", { name: "通知" });
    await userEvent.selectOptions(select, "off");
    await expect(select).toHaveValue("off");
  }),
};

export const Off: Story = {
  decorators: [
    (Story) => (
      <FakeDesktop enabled={false} supported>
        <Story />
      </FakeDesktop>
    ),
  ],
  play: playForImplementation(async () => {
    await expect(await screen.findByRole("combobox", { name: "通知" })).toHaveValue("off");
  }),
};

/** 古い版のアプリ。設定のコマンドがないので、説明だけを出す。 */
export const OlderApp: Story = {
  decorators: [
    (Story) => (
      <FakeDesktop enabled supported={false}>
        <Story />
      </FakeDesktop>
    ),
  ],
  play: playForImplementation(async () => {
    await expect(await screen.findByText("OS の通知")).toBeVisible();
  }),
};
