import { createFileRoute } from "@tanstack/react-router";
import { AppConsoleList } from "../components/apps/console/AppConsoleList";

export const Route = createFileRoute("/_shell/apps/")({ component: AppConsoleList });
