import { describe, expect, it, vi } from "vite-plus/test";

import type { ApiClient } from "./client";
import { conversionBlockers, encryptChannel } from "./encryptChannel";

describe("encryptChannel", () => {
  it("posts to the channel's encrypt endpoint without a body", async () => {
    const post = vi.fn(async () => ({ data: { id: "c-1", encrypted: true }, txid: "7" }));
    const client = { post } as unknown as ApiClient;

    const response = await encryptChannel(client, "c-1");

    expect(post).toHaveBeenCalledWith("/api/channels/c-1/encrypt");
    expect(response).toEqual({ data: { id: "c-1", encrypted: true }, txid: "7" });
  });
});

describe("conversionBlockers", () => {
  it("keeps only the known blockers in a fixed order", () => {
    expect(
      conversionBlockers({ code: "E2EE_CONVERSION_BLOCKED", blockers: ["webhooks", "x", "apps"] }),
    ).toEqual(["apps", "webhooks"]);
    expect(conversionBlockers({ blockers: ["encrypted_dm"] })).toEqual(["encrypted_dm"]);
  });

  it("returns nothing for other bodies", () => {
    expect(conversionBlockers(null)).toEqual([]);
    expect(conversionBlockers({ blockers: "apps" })).toEqual([]);
    expect(conversionBlockers("apps")).toEqual([]);
  });
});
