import { describe, expect, it } from "vitest";
import { accountProfileUrl } from "./accountUrl";

describe("accountProfileUrl", () => {
  it("issuer の origin のプロフィール設定へ、戻り先付きで向ける", () => {
    expect(accountProfileUrl("https://account.zli.works/api/auth", "https://zoo.example/")).toBe(
      "https://account.zli.works/settings/profile?return_to=https%3A%2F%2Fzoo.example%2F",
    );
  });

  it("issuer が URL でなければ null", () => {
    expect(accountProfileUrl("not a url", "https://zoo.example/")).toBeNull();
  });
});
