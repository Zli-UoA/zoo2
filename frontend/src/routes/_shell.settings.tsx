import { createFileRoute } from "@tanstack/react-router";
import { ZooSettings } from "../components/layout/ZooSettings";

export const Route = createFileRoute("/_shell/settings")({
  validateSearch: (
    search: Record<string, unknown>,
  ): { tab?: "general" | "apps" | "members"; channel?: string } => ({
    tab: search.tab === "apps" || search.tab === "members" ? search.tab : "general",
    channel: typeof search.channel === "string" ? search.channel : undefined,
  }),
  component: ZooSettings,
});
