import { describe, expect, it } from "vite-plus/test";

import { ApiError } from "../lib/api/client";
import { describeApiError } from "./useChannelAdmin";

describe("describeApiError", () => {
  it("prefers Japanese messages for permission, not-found, and state errors", () => {
    expect(
      describeApiError(
        new ApiError(403, "Request failed", {
          error: "You do not have permission to kick this member",
        }),
      ),
    ).toBe("この操作を行う権限がありません。");
    expect(
      describeApiError(new ApiError(404, "Request failed", { error: "Message not found" })),
    ).toBe("対象が見つかりませんでした。");
    expect(
      describeApiError(
        new ApiError(400, "Request failed", { error: "Cannot kick member in an archived channel" }),
      ),
    ).toBe("この操作は現在の状態では実行できません。");
  });

  it("uses the backend error body for other statuses", () => {
    expect(
      describeApiError(new ApiError(500, "Request failed", { error: "Database operation failed" })),
    ).toBe("Database operation failed");
  });

  it("falls back to the error message when there is no body", () => {
    expect(describeApiError(new Error("boom"))).toBe("boom");
    expect(describeApiError("boom")).toBe("boom");
  });
});
