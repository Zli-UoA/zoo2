import { describe, expect, it, vi } from "vite-plus/test";

import {
  cachedAttachmentUrl,
  forgetAttachmentUrl,
  requestAttachmentUrl,
  resolveAttachmentUrl,
} from "./attachmentUrls";
import type { ApiClient } from "./client";

function clientReturning(urls: string[]) {
  const requestWithTx = vi.fn(async () => ({
    data: { download_url: urls.shift() ?? "https://storage.example/none" },
    txid: null,
  }));
  return { client: { requestWithTx } as unknown as ApiClient, requestWithTx };
}

// 期限（X-Amz-Date + X-Amz-Expires）が十分先の署名付き URL。
function signedUrl(name: string, expiresSeconds = 3600) {
  const now = new Date();
  const amzDate = now
    .toISOString()
    .replaceAll(/[-:]/g, "")
    .replace(/\.\d{3}/, "");
  return `https://storage.example/${name}?X-Amz-Date=${amzDate}&X-Amz-Expires=${expiresSeconds}`;
}

describe("attachment URL cache", () => {
  it("shares in-flight requests and caches the URL per scope", async () => {
    const scope = {};
    const url = signedUrl("a");
    const { client, requestWithTx } = clientReturning([url]);

    const [first, second] = await Promise.all([
      resolveAttachmentUrl(client, scope, "a"),
      resolveAttachmentUrl(client, scope, "a"),
    ]);
    expect(first).toBe(url);
    expect(second).toBe(url);
    expect(requestWithTx).toHaveBeenCalledTimes(1);
    expect(requestWithTx).toHaveBeenCalledWith("/api/attachments/a/url", { method: "GET" });

    await expect(resolveAttachmentUrl(client, scope, "a")).resolves.toBe(url);
    expect(requestWithTx).toHaveBeenCalledTimes(1);
    expect(cachedAttachmentUrl({}, "a")).toBeUndefined();
  });

  it("fetches again after the URL is forgotten or close to expiry", async () => {
    const scope = {};
    const fresh = signedUrl("fresh");
    const { client, requestWithTx } = clientReturning([signedUrl("old", 1), fresh, fresh]);

    await requestAttachmentUrl(client, scope, "a");
    // 期限が近い URL は使わない。
    await expect(resolveAttachmentUrl(client, scope, "a")).resolves.toBe(fresh);
    forgetAttachmentUrl(scope, "a");
    expect(cachedAttachmentUrl(scope, "a")).toBeUndefined();
    await resolveAttachmentUrl(client, scope, "a");
    expect(requestWithTx).toHaveBeenCalledTimes(3);
  });
});
