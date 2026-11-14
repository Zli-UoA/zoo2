import { createFileRoute } from "@tanstack/react-router";
import { ZooSettings } from "../components/layout/ZooSettings";

export const SETTINGS_TABS = [
  "general",
  "call",
  "encryption",
  "apps",
  "members",
  "activitypub",
  "emojis",
  "slack",
  "slack-import",
] as const;
export type SettingsTab = (typeof SETTINGS_TABS)[number];

/** タブの中で移る先。`devices` は「暗号化」タブの端末のセクション。 */
export type SettingsAnchor = "devices";

export interface SettingsSearch {
  tab?: SettingsTab;
  section?: SettingsAnchor;
  channel?: string;
  slack?: string;
  reason?: string;
  code?: string;
  state?: string;
}

export const Route = createFileRoute("/_shell/settings")({
  validateSearch: (search: Record<string, unknown>): SettingsSearch => ({
    // 「端末」タブは「暗号化」タブのセクションになった。古いリンクは、そのセクションへ移す。
    tab:
      search.tab === "devices"
        ? "encryption"
        : (SETTINGS_TABS as readonly unknown[]).includes(search.tab)
          ? (search.tab as SettingsTab)
          : "general",
    section: search.tab === "devices" || search.section === "devices" ? "devices" : undefined,
    channel: typeof search.channel === "string" ? search.channel : undefined,
    // Slack の OAuth から戻ったときの結果 (SlackLinkPanel が読んで連携を完了し、消す)
    slack: typeof search.slack === "string" ? search.slack : undefined,
    reason: typeof search.reason === "string" ? search.reason : undefined,
    code: typeof search.code === "string" ? search.code : undefined,
    state: typeof search.state === "string" ? search.state : undefined,
  }),
  component: ZooSettings,
});
