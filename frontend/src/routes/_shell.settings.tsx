import { createFileRoute } from "@tanstack/react-router";
import { ZooSettings } from "../components/layout/ZooSettings";
import { normalizeSettingsSearch } from "../components/layout/settingsSearch";

export const Route = createFileRoute("/_shell/settings")({
  validateSearch: normalizeSettingsSearch,
  component: ZooSettings,
});
