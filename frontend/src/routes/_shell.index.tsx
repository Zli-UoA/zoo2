import { createFileRoute } from "@tanstack/react-router";

import { WelcomePanel } from "../components/chat/WelcomePanel";

export const Route = createFileRoute("/_shell/")({ component: Home });

function Home() {
  return <WelcomePanel />;
}
