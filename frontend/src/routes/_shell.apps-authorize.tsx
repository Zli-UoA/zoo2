import { createFileRoute } from "@tanstack/react-router";
import { AppConsent } from "../components/chat/AppConsent";
export const Route = createFileRoute("/_shell/apps-authorize")({ component: AppConsent });
