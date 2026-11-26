import { createFileRoute } from "@tanstack/react-router";

import {
  parseSearchRouteParams,
  type SearchRouteParams,
} from "../components/chat/search/globalSearch";
import { SearchScreen } from "../components/chat/search/SearchScreen";

export const Route = createFileRoute("/_shell/search")({
  validateSearch: (search: Record<string, unknown>): SearchRouteParams =>
    parseSearchRouteParams(search),
  component: SearchScreen,
});
