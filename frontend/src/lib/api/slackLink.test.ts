import { describe, expect, it, vi } from "vite-plus/test";

import type { ApiClient } from "./client";
import {
  completeSlackLink,
  readSlackLinkResult,
  slackLinkErrorReason,
  startSlackLink,
  unlinkSlack,
} from "./slackLink";

describe("readSlackLinkResult", () => {
  it("reads the result Slack's OAuth callback redirects back with", () => {
    expect(readSlackLinkResult({ code: "c", state: "s" })).toEqual({
      status: "authorized",
      code: "c",
      state: "s",
    });
    expect(readSlackLinkResult({ slack: "error", reason: "team_mismatch" })).toEqual({
      status: "error",
      reason: "team_mismatch",
    });
    expect(readSlackLinkResult({ slack: "error" })).toEqual({ status: "error", reason: "" });
    expect(readSlackLinkResult({ tab: "slack" })).toBeNull();
  });
});

describe("Slack link requests", () => {
  it("asks the backend for the authorize URL and unlinks with DELETE", async () => {
    const request = vi.fn(async () => ({ authorize_url: "https://slack.com/oauth/v2/authorize" }));
    const client = { request } as unknown as ApiClient;
    await expect(startSlackLink(client)).resolves.toEqual({
      authorize_url: "https://slack.com/oauth/v2/authorize",
    });
    expect(request).toHaveBeenCalledWith("/api/slack/oauth/start", { method: "POST" });
    await completeSlackLink(client, "c", "s");
    expect(request).toHaveBeenLastCalledWith("/api/slack/oauth/complete", {
      method: "POST",
      body: { code: "c", state: "s" },
    });
    await unlinkSlack(client);
    expect(request).toHaveBeenLastCalledWith("/api/slack/link", { method: "DELETE" });
  });
});

describe("slackLinkErrorReason", () => {
  it("maps response statuses to messages", () => {
    expect(slackLinkErrorReason(400)).toBe("invalid_state");
    expect(slackLinkErrorReason(403)).toBe("team_mismatch");
    expect(slackLinkErrorReason(409)).toBe("already_linked");
    expect(slackLinkErrorReason(502)).toBe("failed");
    expect(slackLinkErrorReason(null)).toBe("failed");
  });
});
