import { http, HttpResponse } from "msw";
import type { BubbleChatEntry } from "../src/hooks/useBubbleChat";

export const storyBubbleEntries: BubbleChatEntry[] = [
  {
    user_id: "user-bob",
    user_name: "bob",
    display_name: "Bob",
    content: "今日のミーティングは何時からでしたっけ？",
  },
  {
    user_id: "user-carol",
    user_name: "carol",
    display_name: "Carol",
    content: "15時からだった気がします！",
  },
];
export const bubbleChatHandlers = ["/bubble", "/bubble-chat"].map((path) =>
  http.get(`*/api${path}`, () => HttpResponse.json(storyBubbleEntries)),
);
