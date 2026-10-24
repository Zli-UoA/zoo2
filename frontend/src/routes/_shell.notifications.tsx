import { createFileRoute } from "@tanstack/react-router";
import { NotificationsScreen } from "../components/chat/NotificationsScreen";

export const Route = createFileRoute("/_shell/notifications")({ component: NotificationsScreen });
