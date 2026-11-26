import { createFileRoute } from "@tanstack/react-router";

import { CatchUpScreen } from "../components/chat/CatchUpScreen";

export const Route = createFileRoute("/_shell/catch-up")({ component: CatchUpScreen });
