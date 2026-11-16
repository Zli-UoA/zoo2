import { describe, expect, it, vi } from "vite-plus/test";

import {
  currentShellEntry,
  fetchDeployedShellEntry,
  isShellOutdated,
  parseShellEntry,
} from "./shell-update";

const html = (entry: string) =>
  `<!doctype html><html><head><script type="module" crossorigin src="${entry}"></script></head></html>`;

const respond = (body: string, ok = true) =>
  vi.fn(async () => ({ ok, text: async () => body }) as Response) as unknown as typeof fetch;

describe("parseShellEntry", () => {
  it("入口のスクリプトを取り出す", () => {
    expect(parseShellEntry(html("/assets/index-AbC123.js"))).toBe("/assets/index-AbC123.js");
  });

  it("入口がなければ null", () => {
    expect(parseShellEntry("<html></html>")).toBeNull();
  });
});

describe("currentShellEntry", () => {
  const doc = (src: string | null) =>
    ({
      baseURI: "https://zli.chat/channels/1",
      querySelector: () => (src === null ? null : { src: `https://zli.chat${src}` }),
    }) as unknown as Pick<Document, "querySelector" | "baseURI">;

  it("文書の入口のスクリプトのパスを返す", () => {
    expect(currentShellEntry(doc("/assets/index-old.js"))).toBe("/assets/index-old.js");
  });

  it("入口のスクリプトがなければ (開発用のサーバー) null", () => {
    expect(currentShellEntry(doc(null))).toBeNull();
  });
});

describe("isShellOutdated", () => {
  it("サーバーの入口が違えば true", async () => {
    const fetchImpl = respond(html("/assets/index-new.js"));
    expect(await isShellOutdated("/assets/index-old.js", fetchImpl)).toBe(true);
    expect(fetchImpl).toHaveBeenCalledWith(
      "/",
      expect.objectContaining({ cache: "no-store", credentials: "same-origin" }),
    );
  });

  it("同じなら false", async () => {
    expect(
      await isShellOutdated("/assets/index-old.js", respond(html("/assets/index-old.js"))),
    ).toBe(false);
  });

  it("開発用のサーバー、失敗、入口のない応答では false", async () => {
    expect(await isShellOutdated(null, respond(html("/assets/index-new.js")))).toBe(false);
    expect(await isShellOutdated("/assets/index-old.js", respond("", false))).toBe(false);
    expect(await isShellOutdated("/assets/index-old.js", respond("<html></html>"))).toBe(false);
    const failing = vi.fn(async () => {
      throw new TypeError("offline");
    }) as unknown as typeof fetch;
    expect(await isShellOutdated("/assets/index-old.js", failing)).toBe(false);
  });

  it("本文の受信が止まったら、時間で打ち切る", async () => {
    const stalled = vi.fn(
      async (_input: RequestInfo | URL, init?: RequestInit) =>
        ({
          ok: true,
          text: () =>
            new Promise<string>((_resolve, reject) =>
              init?.signal?.addEventListener("abort", () => reject(new Error("aborted"))),
            ),
        }) as Response,
    ) as unknown as typeof fetch;
    await expect(fetchDeployedShellEntry(stalled, 10)).rejects.toThrow("aborted");
  });
});
