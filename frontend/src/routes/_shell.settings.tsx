import { createFileRoute } from "@tanstack/react-router";
import { ZooSettings } from "../components/layout/ZooSettings";

export const SETTINGS_TABS = ["general", "apps", "members", "slack", "slack-import"] as const;
export type SettingsTab = (typeof SETTINGS_TABS)[number];

export const Route = createFileRoute("/_shell/settings")({
  validateSearch: (
    search: Record<string, unknown>,
  ): {
    tab?: SettingsTab;
    channel?: string;
    slack?: string;
    reason?: string;
    code?: string;
    state?: string;
  } => ({
    tab: (SETTINGS_TABS as readonly unknown[]).includes(search.tab)
      ? (search.tab as SettingsTab)
      : "general",
    channel: typeof search.channel === "string" ? search.channel : undefined,
    // Slack の OAuth から戻ったときの結果 (SlackLinkPanel が読んで連携を完了し、消す)
    slack: typeof search.slack === "string" ? search.slack : undefined,
    reason: typeof search.reason === "string" ? search.reason : undefined,
    code: typeof search.code === "string" ? search.code : undefined,
    state: typeof search.state === "string" ? search.state : undefined,
  }),
  component: ZooSettings,
});
