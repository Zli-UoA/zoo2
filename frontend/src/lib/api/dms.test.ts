import { describe, expect, it, vi } from "vite-plus/test";

import type { ApiClient } from "./client";
import { createOrGetDm } from "./dms";

describe("createOrGetDm", () => {
  it("posts the selected targets to the DM endpoint", async () => {
    const post = vi.fn(async () => ({ data: { id: "dm-1" }, txid: "42" }));
    const client = { post } as unknown as ApiClient;

    const response = await createOrGetDm(client, ["user-1", "user-2"]);

    expect(post).toHaveBeenCalledWith("/api/dms", {
      target_user_ids: ["user-1", "user-2"],
    });
    expect(response).toEqual({ data: { id: "dm-1" }, txid: "42" });
  });
});
