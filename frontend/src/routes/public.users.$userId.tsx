import { createFileRoute } from "@tanstack/react-router";
import { useState } from "react";
import { PublicProfile } from "../components/public/PublicProfile";
import { ComposeRuntime } from "../components/experimental/ComposeFrontend";
import { selectedFrontend } from "../lib/experimentalFrontend";
import { resolvePublicConfig } from "../lib/config";

export const Route = createFileRoute("/public/users/$userId")({
  validateSearch: (search: Record<string, unknown>): { before?: string } => ({
    before: typeof search.before === "string" ? search.before : undefined,
  }),
  component: PublicProfileRoute,
});
function PublicProfileRoute() {
  const { userId } = Route.useParams();
  const { before } = Route.useSearch();
  const [implementation] = useState(selectedFrontend);
  const [config] = useState(resolvePublicConfig);
  return implementation === "compose" ? (
    <ComposeRuntime config={config} anonymous />
  ) : (
    <PublicProfile userId={userId} before={before} />
  );
}
