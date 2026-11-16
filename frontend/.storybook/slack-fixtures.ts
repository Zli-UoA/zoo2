import { http, HttpResponse } from "msw";

export const slackNotLinkedHandler = http.get("*/api/slack/link", () =>
  HttpResponse.json({ configured: true, link: null }),
);
