import { createFileRoute } from "@tanstack/react-router";
import { HomeScreen } from "../components/home/HomeScreen";
export const Route = createFileRoute("/_shell/home_/users/$actorId")({
  component: ProfilePage,
});
function ProfilePage() {
  const { actorId } = Route.useParams();
  return <HomeScreen key={actorId} actorId={actorId} />;
}
