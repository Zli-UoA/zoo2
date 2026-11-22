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

  it("does not share an isolated load, so cancelling a save does not fail a preview", async () => {
    const cache = createDecryptedAttachmentCache();
    let finishPreview: ((value: Blob) => void) | undefined;
    const previewLoader = vi.fn(
      () =>
        new Promise<Blob>((resolve) => {
          finishPreview = resolve;
        }),
    );
    let failSave: ((error: unknown) => void) | undefined;
    const saveLoader = vi.fn(
      () =>
        new Promise<Blob>((_, reject) => {
          failSave = reject;
        }),
    );

    const preview = cache.load(info("a"), previewLoader);
    const save = cache.load(info("a"), saveLoader, { isolated: true });
    // プレビューが先に始めた取得に、保存は合流しない（中止しても巻き込まない）。
    expect(previewLoader).toHaveBeenCalledTimes(1);
    expect(saveLoader).toHaveBeenCalledTimes(1);

    failSave?.(new EncryptedAttachmentError("download"));
    await expect(save).rejects.toMatchObject({ reason: "download" });
    expect(cache.failed(info("a"))).toBe(false);

    const value = blob(2);
    finishPreview?.(value);
    await expect(preview).resolves.toBe(value);
  });

  it("keeps the result of an isolated load in the cache", async () => {
    const cache = createDecryptedAttachmentCache();
    const value = blob(4);
    await expect(cache.load(info("a"), async () => value, { isolated: true })).resolves.toBe(value);
    // 保存で復号したものを、次にプレビューが開いたときに使える。
    expect(cache.peek(info("a"))).toBe(value);
  });

  it("clears an earlier failure when a parallel isolated load succeeds", async () => {
    const cache = createDecryptedAttachmentCache();
    let failPreview: ((error: unknown) => void) | undefined;
    let finishSave: ((value: Blob) => void) | undefined;
    const previewLoader = vi.fn(
      () =>
        new Promise<Blob>((_, reject) => {
          failPreview = reject;
        }),
    );
    const saveLoader = vi.fn(
      () =>
        new Promise<Blob>((resolve) => {
          finishSave = resolve;
        }),
    );

    const preview = cache.load(info("a"), previewLoader);
    const save = cache.load(info("a"), saveLoader, { isolated: true });
    // プレビューの復号が失敗して記録されたあとに、並行していた保存の復号が成功する。
    failPreview?.(new EncryptedAttachmentError("decrypt"));
    await expect(preview).rejects.toMatchObject({ reason: "decrypt" });
    expect(cache.failed(info("a"))).toBe(true);
    const value = blob(2);
    finishSave?.(value);
    await expect(save).resolves.toBe(value);

    // 失敗の記録を消し、次にプレビューが開いたときは再取得せずにキャッシュから出す。
    expect(cache.failed(info("a"))).toBe(false);
    await expect(cache.load(info("a"), previewLoader)).resolves.toBe(value);
    expect(previewLoader).toHaveBeenCalledTimes(1);
  });

  it("keys on the key material, not only the attachment id", async () => {
    // 同じ attachment_id を別の鍵で指すメッセージに、ほかのメッセージの平文を出さない。
    expect(decryptedAttachmentCacheKey(info("a", "k1"))).not.toBe(
      decryptedAttachmentCacheKey(info("a", "k2")),
    );
    expect(decryptedAttachmentCacheKey(info("a", "k1", "image/png"))).not.toBe(
      decryptedAttachmentCacheKey(info("a", "k1", "text/html")),
    );
    // 大きさの検証は復号のあとなので、本文の size が違えば結果を共有しない。
    expect(decryptedAttachmentCacheKey(info("a"))).not.toBe(
      decryptedAttachmentCacheKey({ ...info("a"), size: 2 }),
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

  it("late isolated failure does not resurrect after a successful blob is evicted", async () => {
    const cache = createDecryptedAttachmentCache({ maxBytes: 4, maxEntries: 1 });
    const decrypt = new EncryptedAttachmentError("decrypt");
    let rejectIsolated: ((error: unknown) => void) | undefined;
    const isolated = new Promise<Blob>((_resolve, reject) => {
      rejectIsolated = reject;
    });
    const signal = new AbortController().signal;

    const isolatedLoad = cache.load(info("a"), () => isolated, { signal, isolated: true });
    await expect(cache.load(info("a"), async () => blob(4))).resolves.toBeInstanceOf(Blob);

    rejectIsolated?.(decrypt);
    await expect(isolatedLoad).rejects.toBe(decrypt);

    await expect(cache.load(info("b"), async () => blob(4))).resolves.toBeInstanceOf(Blob);
    await expect(cache.load(info("a"), async () => blob(3))).resolves.toBeInstanceOf(Blob);
  });
});
