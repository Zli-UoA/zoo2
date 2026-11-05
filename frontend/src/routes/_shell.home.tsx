import { createFileRoute } from "@tanstack/react-router";
import { HomeScreen } from "../components/home/HomeScreen";
export const Route = createFileRoute("/_shell/home")({ component: HomeScreen });
