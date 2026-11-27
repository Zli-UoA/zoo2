import { describe, expect, it, vi } from "vite-plus/test";

import type { ApiClient } from "../api/client";
import { fetchRecipients } from "./megolmApi";

describe("配布先の次ページの検証", () => {
  it.each([1, true, {}, []])("不正な位置 %j を最終ページとして扱わない", async (cursor) => {
    const request = vi.fn().mockResolvedValue({ membership_epoch: 1, next_after: cursor });
    await expect(fetchRecipients({ request } as unknown as ApiClient, "channel", null)).rejects.toThrow(
      "配布先の次ページの位置が不正です",
    );
    expect(request).toHaveBeenCalledTimes(1);
  });

  it("文字列の位置で次のページを取得し、null で終了する", async () => {
    const first = { membership_epoch: 1, next_after: "next" };
    const last = { membership_epoch: 1, next_after: null };
    const request = vi.fn().mockResolvedValueOnce(first).mockResolvedValueOnce(last);
    expect(await fetchRecipients({ request } as unknown as ApiClient, "channel", null)).toEqual([
      first,
      last,
    ]);
    expect(request.mock.calls[1]?.[0]).toContain("after=next&epoch=1");
  });
});
