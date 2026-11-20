import { createFileRoute } from "@tanstack/react-router";

import { NotFoundScreen } from "../components/layout/NotFoundScreen";

/** 既存ルートに一致しないパスは、アプリシェル内の本文に 404 を出す。 */
export const Route = createFileRoute("/_shell/$")({
  component: NotFoundScreen,
});
