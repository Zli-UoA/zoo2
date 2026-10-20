import { describe, expect, it } from "vite-plus/test";

import { ApiError } from "../lib/api/client";
import { createI18n } from "../lib/i18n";
import { describeApiError } from "./useChannelAdmin";

const ja = createI18n("ja").t;
const en = createI18n("en").t;

describe("describeApiError", () => {
  it("prefers translated messages for permission, not-found, and state errors", () => {
    expect(
      describeApiError(
        new ApiError(403, "Request failed", {
          error: "You do not have permission to kick this member",
        }),
        ja,
      ),
    ).toBe("この操作を行う権限がありません。");
    expect(
      describeApiError(new ApiError(404, "Request failed", { error: "Message not found" }), ja),
    ).toBe("対象が見つかりませんでした。");
    expect(
      describeApiError(
        new ApiError(400, "Request failed", { error: "Cannot kick member in an archived channel" }),
        ja,
      ),
    ).toBe("この操作は現在の状態では実行できません。");
  });

  it("uses the selected locale", () => {
    expect(describeApiError(new ApiError(403, "Request failed"), en)).toBe(
      "You do not have permission to do this.",
    );
  });

  it("uses the backend error body for other statuses", () => {
    expect(
      describeApiError(
        new ApiError(500, "Request failed", { error: "Database operation failed" }),
        ja,
      ),
    ).toBe("Database operation failed");
  });

  it("falls back to the error message when there is no body", () => {
    expect(describeApiError(new Error("boom"), ja)).toBe("boom");
    expect(describeApiError("boom", ja)).toBe("boom");
  });
});
