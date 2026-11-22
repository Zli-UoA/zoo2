import { preferenceSurfaces } from "../../../.storybook/preference-surfaces";
import { playForImplementation } from "../../../.storybook/play-for-implementation";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, screen, userEvent } from "storybook/test";
import { DownloadIcon } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";

import {
  DESKTOP_PROTOCOL_VERSION,
  detectDesktop,
  resetDesktopForTest,
  type DesktopDownloadSettings,
} from "../../lib/desktop";
import { SettingRow } from "../ui/control-panel";
import { DesktopDownloadSettings as DesktopDownloadSettingsView } from "./DesktopDownloadSettings";

/** デスクトップ版 (Tauri) の中にいるように見せる。`supported` が false なら古い版のアプリ。 */
function FakeDesktop({
  askEveryTime,
  directory = null,
  supported,
  failSettings = false,
  picked = "C:\\Users\\zoo\\Downloads",
  children,
}: {
  askEveryTime: boolean;
  directory?: string | null;
  supported: boolean;
  // 設定の読み込みだけを、コマンド非対応ではない失敗にする。
  failSettings?: boolean;
  picked?: string;
  children: ReactNode;
}) {
  const [ready, setReady] = useState(false);
  useEffect(() => {
    let current: DesktopDownloadSettings = { ask_every_time: askEveryTime, directory };
    resetDesktopForTest();
    Object.assign(window, {
      __TAURI_INTERNALS__: {
        invoke: async (
          cmd: string,
          args?: {
            settings?: { ask_every_time: boolean; reset_directory?: boolean };
            begin?: unknown;
          },
        ) => {
          if (cmd === "zoo_session")
            return {
              protocol: DESKTOP_PROTOCOL_VERSION,
              ui: "remote",
              api_base_url: "https://zli.chat",
              state: { state: "ready" },
            };
          if (!supported) throw new Error(`command ${cmd} not found`);
          if (cmd === "zoo_download_settings") {
            if (failSettings) throw new Error("設定を読めません");
            return current;
          }
          if (cmd === "zoo_set_download_settings") {
            if (args?.settings) {
              current = {
                ask_every_time: args.settings.ask_every_time,
                directory: args.settings.reset_directory ? null : current.directory,
              };
            }
            return current;
          }
          if (cmd === "zoo_pick_download_directory") {
            current = { ...current, directory: picked };
            return current;
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
  }, [askEveryTime, directory, supported, failSettings, picked]);
  return ready ? <>{children}</> : null;
}

const meta = {
  title: "Features/Preferences/DesktopDownloads",
  component: DesktopDownloadSettingsView,
  parameters: { a11y: { test: "error" } },
  // 行の名前 (「ダウンロード」) は日本語で書いているので、ツールバーのロケールによらず日本語で出す。
  globals: { locale: "ja" },
  decorators: [
    (Story) => (
      <div className={preferenceSurfaces["Features/Preferences/DesktopDownloads"]}>
        {/* 設定画面 (ZooSettings) と同じ行に置く。 */}
        <div className="w-full rounded-lg border">
          <SettingRow
            icon={<DownloadIcon />}
            label="ダウンロード"
            description="添付ファイルの保存先です。保存のたびに場所を確認するか、あらかじめ決めたフォルダーへ保存するかを選べます。"
          >
            <Story />
          </SettingRow>
        </div>
      </div>
    ),
  ],
} satisfies Meta<typeof DesktopDownloadSettingsView>;

export default meta;
type Story = StoryObj<typeof meta>;

/** 既定。保存のたびに OS の「名前を付けて保存」を出す。 */
export const AskEveryTime: Story = {
  decorators: [
    (Story) => (
      <FakeDesktop supported askEveryTime>
        <Story />
      </FakeDesktop>
    ),
  ],
  play: playForImplementation(async () => {
    await expect(await screen.findByRole("switch", { name: "保存のたびに確認する" })).toBeChecked();
    // 「都度確認」がオンのときは、フォルダーは出さない。
    await expect(screen.queryByRole("button", { name: "フォルダーを選ぶ" })).toBeNull();
  }),
};

/** オフにすると、決めたフォルダーへ確認なしで保存する。 */
export const SaveToFolder: Story = {
  decorators: [
    (Story) => (
      <FakeDesktop supported askEveryTime={false} directory={"C:\\Users\\zoo\\Documents"}>
        <Story />
      </FakeDesktop>
    ),
  ],
  play: playForImplementation(async () => {
    await expect(
      await screen.findByRole("switch", { name: "保存のたびに確認する" }),
    ).not.toBeChecked();
    await expect(screen.getByText("C:\\Users\\zoo\\Documents")).toBeVisible();
  }),
};

export const ToggleOff: Story = {
  decorators: [
    (Story) => (
      <FakeDesktop supported askEveryTime>
        <Story />
      </FakeDesktop>
    ),
  ],
  play: playForImplementation(async () => {
    const control = await screen.findByRole("switch", { name: "保存のたびに確認する" });
    // 設定を読むまでは操作できない。読み終わるのを待ってから切り替える。
    await expect(control).toBeEnabled();
    await userEvent.click(control);
    await expect(control).not.toBeChecked();
    // 保存先が無いので、オフにするときに OS のフォルダー選択を出す。
    await expect(await screen.findByText("C:\\Users\\zoo\\Downloads")).toBeVisible();
  }),
};

/** フォルダーは OS の選択ダイアログで決める。 */
export const ChooseFolder: Story = {
  decorators: [
    (Story) => (
      <FakeDesktop supported askEveryTime={false} picked={"D:\\Downloads"}>
        <Story />
      </FakeDesktop>
    ),
  ],
  play: playForImplementation(async () => {
    await userEvent.click(await screen.findByRole("button", { name: "フォルダーを選ぶ" }));
    await expect(await screen.findByText("D:\\Downloads")).toBeVisible();
  }),
};

/** 選んだフォルダーを消すと、毎回確認する設定へ戻る。 */
export const ResetFolder: Story = {
  decorators: [
    (Story) => (
      <FakeDesktop supported askEveryTime={false} directory={"C:\\Users\\zoo\\Documents"}>
        <Story />
      </FakeDesktop>
    ),
  ],
  play: playForImplementation(async () => {
    await userEvent.click(await screen.findByRole("button", { name: "毎回確認する設定に戻す" }));
    await expect(await screen.findByRole("switch", { name: "保存のたびに確認する" })).toBeChecked();
    await expect(screen.queryByRole("button", { name: "フォルダーを選ぶ" })).toBeNull();
  }),
};

/** 古い版のアプリ。保存先のコマンドがないので、実際の保存先を示す。 */
export const OlderApp: Story = {
  decorators: [
    (Story) => (
      <FakeDesktop supported={false} askEveryTime>
        <Story />
      </FakeDesktop>
    ),
  ],
  play: playForImplementation(async () => {
    await expect(await screen.findByText("アプリの既定の保存先")).toBeVisible();
    await expect(screen.getByText(/アプリを更新/)).toBeVisible();
  }),
};

/** 設定を読めなかっただけのとき。古い版と決めつけず、再読み込みを出す。 */
export const LoadError: Story = {
  decorators: [
    (Story) => (
      <FakeDesktop supported failSettings askEveryTime>
        <Story />
      </FakeDesktop>
    ),
  ],
  play: playForImplementation(async () => {
    await expect(await screen.findByText("保存先の設定を読み込めませんでした。")).toBeVisible();
    await expect(screen.getByRole("button", { name: "再読み込み" })).toBeVisible();
    // 更新を促す文言は出さない。
    await expect(screen.queryByText(/アプリを更新/)).toBeNull();
  }),
};

/** 長い保存先でも、操作部分がはみ出さないよう、パスは 1 行に省略する。 */
export const LongPath: Story = {
  decorators: [
    (Story) => (
      <FakeDesktop
        supported
        askEveryTime={false}
        directory={
          "C:\\Users\\zoo\\Documents\\Projects\\2026\\a-very-long-folder-name\\another-folder"
        }
      >
        <Story />
      </FakeDesktop>
    ),
  ],
  play: playForImplementation(async () => {
    await expect(await screen.findByRole("button", { name: "フォルダーを選ぶ" })).toBeVisible();
    await expect(screen.getByRole("button", { name: "毎回確認する設定に戻す" })).toBeVisible();
  }),
};
