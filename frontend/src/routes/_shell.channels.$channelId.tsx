import { createFileRoute } from "@tanstack/react-router";

import { ChannelScreen } from "../components/chat/ChannelScreen";

export const Route = createFileRoute("/_shell/channels/$channelId")({
  validateSearch: (search: Record<string, unknown>): { message?: string } => ({
    message: typeof search.message === "string" ? search.message : undefined,
  }),
  component: ChannelRoute,
});

function ChannelRoute() {
  const { channelId } = Route.useParams();
  const { message } = Route.useSearch();
  return <ChannelScreen channelId={channelId} targetMessageId={message} />;
}
