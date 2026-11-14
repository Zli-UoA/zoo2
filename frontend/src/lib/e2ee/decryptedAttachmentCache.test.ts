import type { AttachmentInfo } from "@zoo/crypto-wasm";
import { describe, expect, it, vi } from "vite-plus/test";

import { EncryptedAttachmentError } from "./attachments";
import {
  createDecryptedAttachmentCache,
  decryptedAttachmentCacheFor,
  decryptedAttachmentCacheKey,
} from "./decryptedAttachmentCache";

function info(id: string, k = "key", mimetype = "image/png"): AttachmentInfo {
  return {
    attachment_id: id,
    name: `${id}.png`,
    mimetype,
    size: 1,
    file: {
      v: "v2",
      key: { kty: "oct", key_ops: ["encrypt", "decrypt"], alg: "A256CTR", k, ext: true },
      iv: "iv",
      hashes: { sha256: "hash" },
    },
  };
}

function blob(bytes: number) {
  return new Blob([new Uint8Array(bytes)]);
}

describe("createDecryptedAttachmentCache", () => {
  it("decrypts once and serves remounts from the cache", async () => {
    const cache = createDecryptedAttachmentCache();
    const loader = vi.fn(async () => blob(4));

    expect(cache.peek(info("a"))).toBeUndefined();
    const [first, second] = await Promise.all([
      cache.load(info("a"), loader),
      cache.load(info("a"), loader),
    ]);
    expect(loader).toHaveBeenCalledTimes(1);
    expect(second).toBe(first);
    // 行が再マウントされても（同じ中身の新しい `AttachmentInfo`）、取得も復号もしない。
    expect(cache.peek(info("a"))).toBe(first);
    await expect(cache.load(info("a"), loader)).resolves.toBe(first);
    expect(loader).toHaveBeenCalledTimes(1);
  });

  it("does not keep failures, so a retry loads again", async () => {
    const cache = createDecryptedAttachmentCache();
    const loader = vi
      .fn<(info: AttachmentInfo) => Promise<Blob>>()
      .mockRejectedValueOnce(new Error("download"))
      .mockResolvedValueOnce(blob(1));

    await expect(cache.load(info("a"), loader)).rejects.toThrow("download");
    expect(cache.peek(info("a"))).toBeUndefined();
    await expect(cache.load(info("a"), loader)).resolves.toBeInstanceOf(Blob);
    expect(loader).toHaveBeenCalledTimes(2);
  });

  it("remembers decryption failures until an explicit retry", async () => {
    const cache = createDecryptedAttachmentCache();
    const loader = vi
      .fn<(info: AttachmentInfo) => Promise<Blob>>()
      .mockRejectedValueOnce(new EncryptedAttachmentError("decrypt"))
      .mockResolvedValueOnce(blob(1));

    await expect(cache.load(info("a"), loader)).rejects.toMatchObject({ reason: "decrypt" });
    expect(cache.failed(info("a"))).toBe(true);
    // 再マウントでは取り直さない（ハッシュの合わない大きな添付を何度も取得しない）。
    await expect(cache.load(info("a"), loader)).rejects.toMatchObject({ reason: "decrypt" });
    expect(loader).toHaveBeenCalledTimes(1);
    await expect(cache.load(info("a"), loader, { retry: true })).resolves.toBeInstanceOf(Blob);
    expect(cache.failed(info("a"))).toBe(false);
    expect(loader).toHaveBeenCalledTimes(2);
  });

  it("keys on the key material, not only the attachment id", async () => {
    // 同じ attachment_id を別の鍵で指すメッセージに、ほかのメッセージの平文を出さない。
    expect(decryptedAttachmentCacheKey(info("a", "k1"))).not.toBe(
      decryptedAttachmentCacheKey(info("a", "k2")),
    );
    expect(decryptedAttachmentCacheKey(info("a", "k1", "image/png"))).not.toBe(
      decryptedAttachmentCacheKey(info("a", "k1", "text/html")),
    );
    const cache = createDecryptedAttachmentCache();
    await cache.load(info("a", "k1"), async () => blob(1));
    expect(cache.peek(info("a", "k2"))).toBeUndefined();
  });

  it("evicts the least recently used entries past the byte and entry limits", async () => {
    const cache = createDecryptedAttachmentCache({ maxBytes: 10, maxEntries: 3 });
    await cache.load(info("a"), async () => blob(4));
    await cache.load(info("b"), async () => blob(4));
    // a を使い直すと、b が最も古くなる。
    expect(cache.peek(info("a"))).toBeDefined();
    await cache.load(info("c"), async () => blob(4));
    expect(cache.peek(info("b"))).toBeUndefined();
    expect(cache.peek(info("a"))).toBeDefined();
    expect(cache.peek(info("c"))).toBeDefined();

    // 上限より大きいものは置かない（ほかを追い出さない）。
    await cache.load(info("big"), async () => blob(11));
    expect(cache.peek(info("big"))).toBeUndefined();
    expect(cache.peek(info("a"))).toBeDefined();

    const small = createDecryptedAttachmentCache({ maxBytes: 100, maxEntries: 2 });
    for (const id of ["x", "y", "z"]) await small.load(info(id), async () => blob(1));
    expect(small.peek(info("x"))).toBeUndefined();
    expect(small.peek(info("z"))).toBeDefined();
  });

  it("shares one cache per scope", () => {
    const worker = {};
    expect(decryptedAttachmentCacheFor(worker)).toBe(decryptedAttachmentCacheFor(worker));
    expect(decryptedAttachmentCacheFor({})).not.toBe(decryptedAttachmentCacheFor(worker));
  });
});
