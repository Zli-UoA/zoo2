import { createFileRoute } from "@tanstack/react-router";

import { ChannelScreen } from "../components/chat/ChannelScreen";

export const Route = createFileRoute("/_shell/channels/$channelId")({
  component: ChannelRoute,
});

function ChannelRoute() {
  const { channelId } = Route.useParams();
  return <ChannelScreen channelId={channelId} />;
}
