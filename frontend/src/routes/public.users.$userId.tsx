import { createFileRoute } from "@tanstack/react-router";
import { PublicProfile } from "../components/public/PublicProfile";

export const Route = createFileRoute("/public/users/$userId")({
  validateSearch: (search: Record<string, unknown>): { before?: string } => ({
    before: typeof search.before === "string" ? search.before : undefined,
  }),
  component: PublicProfileRoute,
});
function PublicProfileRoute() {
  const { userId } = Route.useParams();
  const { before } = Route.useSearch();
  return <PublicProfile userId={userId} before={before} />;
}
