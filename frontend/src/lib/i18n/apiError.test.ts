import { describe, expect, it } from "vite-plus/test";

import { ApiError } from "../api/client";
import { createI18n } from "./index";
import { localizeApiError } from "./apiError";

const ja = createI18n("ja").t;
const en = createI18n("en").t;

describe("localizeApiError", () => {
  it("explains a duplicate channel name on create and rename", () => {
    const conflict = { error: "Resource already exists or unique constraint was violated" };
    expect(
      localizeApiError(
        new ApiError(409, "Request failed", conflict, {
          path: "/api/channels",
          method: "POST",
        }),
        ja,
      ),
    ).toBe("同名のチャンネルが既に存在します。");
    expect(
      localizeApiError(
        new ApiError(409, "Request failed", conflict, {
          path: "/api/channels/channel-1",
          method: "PATCH",
        }),
        en,
      ),
    ).toBe("A channel with this name already exists.");
  });

  it("uses the generic conflict message for other 409 responses", () => {
    expect(
      localizeApiError(
        new ApiError(
          409,
          "Request failed",
          { error: "Already a member" },
          {
            path: "/api/channels/channel-1/join",
            method: "POST",
          },
        ),
        ja,
      ),
    ).toBe("操作が競合しました。最新の状態を確認してもう一度お試しください。");
  });

  it.each([
    [400, "入力内容を確認してください。"],
    [401, "ログインの有効期限が切れました。もう一度ログインしてください。"],
    [403, "この操作を行う権限がありません。"],
    [404, "対象が見つかりませんでした。"],
    [429, "リクエストが多すぎます。しばらく待ってからもう一度お試しください。"],
    [500, "サーバーでエラーが発生しました。しばらく待ってからもう一度お試しください。"],
    [418, "操作に失敗しました。もう一度お試しください。"],
  ])("localizes status %i without exposing the backend message", (status, expected) => {
    expect(
      localizeApiError(
        new ApiError(status, "Request failed", { error: "raw backend message" }),
        ja,
      ),
    ).toBe(expected);
  });

  it("localizes network and unknown failures", () => {
    expect(localizeApiError(new TypeError("Failed to fetch"), ja)).toBe(
      "サーバーに接続できませんでした。接続を確認してもう一度お試しください。",
    );
    expect(localizeApiError(new Error("boom"), ja)).toBe(
      "操作に失敗しました。もう一度お試しください。",
    );
    expect(localizeApiError("boom", en)).toBe("The action failed. Try again.");
  });
});
