import { http, HttpResponse } from "msw";
import type { HomePost } from "../src/components/home/HomeScreen";
export const homePosts: HomePost[] = [
  {
    id: "home-post",
    user_id: "user-alice",
    user_name: "alice",
    display_name: "Alice",
    content: "Homeから最初の公開投稿です。\nみんなの近況がここに流れます。",
    created_at: "2026-11-01T10:00:00Z",
  },
  {
    id: "home-long",
    user_id: "user-bob",
    user_name: "bob",
    display_name: "とても長い名前のユーザー".repeat(5),
    content: "https://example.com/" + "long-path".repeat(25),
    created_at: "2026-11-01T09:00:00Z",
  },
];

export const homeHandlers = [
  http.get("*/api/home", ({ request }) => {
    const timeline = new URL(request.url).searchParams.get("timeline") ?? "home";
    return HttpResponse.json(
      timeline === "home" ? homePosts.filter((post) => post.user_id === "user-alice") : homePosts,
    );
  }),
];
