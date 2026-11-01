import { createFileRoute } from "@tanstack/react-router";
import { ZooSettings } from "../components/layout/ZooSettings";

export const SETTINGS_TABS = ["general", "apps", "members", "activitypub", "slack-import"] as const;
export type SettingsTab = (typeof SETTINGS_TABS)[number];

export const Route = createFileRoute("/_shell/settings")({
  validateSearch: (search: Record<string, unknown>): { tab?: SettingsTab; channel?: string } => ({
    tab: (SETTINGS_TABS as readonly unknown[]).includes(search.tab)
      ? (search.tab as SettingsTab)
      : "general",
    channel: typeof search.channel === "string" ? search.channel : undefined,
  }),
  component: ZooSettings,
});
