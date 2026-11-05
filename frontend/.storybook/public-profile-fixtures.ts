import type { PublicProfileData } from "../src/components/public/PublicProfile";

export const publicProfile: PublicProfileData = {
  account: {
    id: "alice",
    user_name: "alice",
    display_name: "Alice / アリス",
    summary: "こんにちは！ **Zoo** から発信しています。\nActivityPubでつながりましょう。",
    actor_id: "https://zoo.example/api/activitypub/users/alice",
    url: "https://zoo.example/public/users/alice",
  },
  posts: [
    {
      id: "p2",
      content: "公開の連合投稿です。 **太字** と :cat: とリンク https://example.com",
      created_at: "2026-11-02T03:34:56Z",
    },
    {
      id: "p1",
      content: "メイン画面と同じMFM表示です。\n```js\nconst zoo = true;\n```",
      created_at: "2026-11-01T03:34:56Z",
    },
  ],
  next_cursor: "p1",
};
export const longPublicProfile: PublicProfileData = {
  ...publicProfile,
  account: {
    ...publicProfile.account,
    display_name: "猫".repeat(100),
    summary: "https://example.com/".repeat(30),
  },
  posts: [{ ...publicProfile.posts[0]!, content: "https://example.com/".repeat(100) }],
  next_cursor: null,
};

export const imagePublicProfile: PublicProfileData = {
  ...publicProfile,
  posts: [
    {
      ...publicProfile.posts[0]!,
      content: "公開プロフィールの画像投稿です。",
      media: [
        {
          url: "/api/home/images/00000000-0000-4000-8000-000000000001",
          name: "profile-photo.png",
          media_type: "image/png",
        },
      ],
    },
    {
      ...publicProfile.posts[1]!,
      content: "",
      media: [
        {
          url: "/api/home/images/00000000-0000-4000-8000-000000000002",
          name: "image-only.png",
          media_type: "image/png",
        },
      ],
    },
  ],
  next_cursor: null,
};
