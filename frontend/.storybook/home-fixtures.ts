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

export const visibilityPosts: HomePost[] = ["public", "unlisted", "followers", "local"].map(
  (visibility, index) => ({
    ...homePosts[0]!,
    id: `visibility-${visibility}`,
    apub: visibility !== "local",
    visibility,
    content: `visibility-${visibility}`,
    created_at: `2026-11-01T${String(10 - index).padStart(2, "0")}:00:00Z`,
  }),
);

export const homeImageOnlyPost: HomePost = {
  ...homePosts[0]!,
  id: "home-image-only",
  content: "",
  media: [
    {
      url: "/api/home/images/11111111-1111-4111-8111-111111111111",
      name: "home-photo.png",
      media_type: "image/png",
    },
  ],
  channel_id: "channel-general",
  attachments: [
    {
      id: "home-image-attachment",
      file_name: "home-photo.png",
      file_size: 184320,
      content_type: "image/png",
    },
  ],
};

export const homeImageBase64 =
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
export const homeImageId = "11111111-1111-4111-8111-111111111111";
export const homeImagePost: HomePost = {
  ...homePosts[0]!,
  content: "",
  media: [{ url: `/api/home/images/${homeImageId}`, name: "景色.png", media_type: "image/png" }],
};
export const homeImageHandler = http.get("*/api/home/images/:id", () =>
  HttpResponse.arrayBuffer(
    Uint8Array.from(atob(homeImageBase64), (char) => char.charCodeAt(0)).buffer,
    { headers: { "Content-Type": "image/png" } },
  ),
);
