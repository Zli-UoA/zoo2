import { createFileRoute } from "@tanstack/react-router";

import { ChannelScreen } from "../components/chat/ChannelScreen";

export const Route = createFileRoute("/_shell/channels/$channelId")({
  validateSearch: (search: Record<string, unknown>): { message?: string; note?: string } => ({
    message: typeof search.message === "string" ? search.message : undefined,
    note: typeof search.note === "string" ? search.note : undefined,
  }),
  component: ChannelRoute,
});

function ChannelRoute() {
  const { channelId } = Route.useParams();
  const { message } = Route.useSearch();
  const { note } = Route.useSearch();
  const navigate = Route.useNavigate();
  return (
    <ChannelScreen
      channelId={channelId}
      targetMessageId={message}
      noteId={note}
      onNoteChange={(id) => {
        void navigate({ search: (previous) => ({ ...previous, note: id }) });
      }}
    />
  );
}
