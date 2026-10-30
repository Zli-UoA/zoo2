import { createFileRoute } from "@tanstack/react-router";
import { AppsDialog } from "../components/chat/AppsDialog";
export const Route = createFileRoute("/_shell/apps/$appId")({ component: AppPage });
function AppPage() {
  const { appId } = Route.useParams();
  const navigate = Route.useNavigate();
  return <AppsDialog initialId={appId} onClose={() => void navigate({ to: "/" })} />;
}
