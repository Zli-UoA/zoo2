import { createFileRoute } from "@tanstack/react-router";
import { BubbleChatScreen } from "../components/chat/BubbleChatScreen";

export const Route = createFileRoute("/_shell/bubble-chat")({
  component: BubbleChatScreen,
});
