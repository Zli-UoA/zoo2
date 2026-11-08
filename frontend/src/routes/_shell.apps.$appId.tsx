import { createFileRoute } from "@tanstack/react-router";
import { AppConsole } from "../components/apps/console/AppConsole";
import { type ConsoleSection, parseSection } from "../components/apps/console/model";

export const Route = createFileRoute("/_shell/apps/$appId")({
  validateSearch: (search: Record<string, unknown>): { section: ConsoleSection } => ({
    section: parseSection(search.section),
  }),
  component: AppPage,
});
function AppPage() {
  const { appId } = Route.useParams();
  const { section } = Route.useSearch();
  return <AppConsole key={appId} appId={appId} section={section} />;
}
