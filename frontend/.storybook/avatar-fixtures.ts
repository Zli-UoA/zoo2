import { http, HttpResponse } from "msw";
import { homeImageBase64 } from "./home-fixtures";
import { storyUsers } from "./fixtures";

export const userAvatarUrl = "https://avater.zliusercontent.com/story-user.png";
export const avatarUsers = storyUsers.map((user) => ({ ...user, avatar_url: userAvatarUrl }));
export const userAvatarHandler = http.get(userAvatarUrl, () =>
  HttpResponse.arrayBuffer(
    Uint8Array.from(atob(homeImageBase64), (char) => char.charCodeAt(0)).buffer,
    {
      headers: { "Content-Type": "image/png" },
    },
  ),
);
