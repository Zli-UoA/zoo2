import { createFileRoute } from "@tanstack/react-router";
import { HomeScreen } from "../components/home/HomeScreen";
export const Route = createFileRoute("/_shell/home_/posts/$postId")({
  component: PostPage,
});
function PostPage() {
  const { postId } = Route.useParams();
  return <HomeScreen key={postId} postId={postId} />;
}
