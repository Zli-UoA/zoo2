import { describe, expect, it } from "vite-plus/test";

import { abortable } from "./abortable";

describe("abortable", () => {
  it("returns the value while the signal is not aborted", async () => {
    await expect(abortable(Promise.resolve("ok"), new AbortController().signal)).resolves.toBe(
      "ok",
    );
  });

  it("ends the wait without cancelling the shared promise", async () => {
    const controller = new AbortController();
    let resolveShared!: (value: string) => void;
    const shared = new Promise<string>((resolve) => (resolveShared = resolve));

    const waiting = abortable(shared, controller.signal);
    controller.abort("stopped");
    await expect(waiting).rejects.toBe("stopped");

    resolveShared("shared result");
    await expect(shared).resolves.toBe("shared result");
  });

  it("handles a later failure of the shared promise even when already aborted", async () => {
    const reason = new Error("stopped");
    let rejectShared!: (error: Error) => void;
    const shared = new Promise<string>((_, reject) => (rejectShared = reject));

    await expect(abortable(shared, AbortSignal.abort(reason))).rejects.toBe(reason);
    // ここで拒否しても、共有の Promise に付けたハンドラーが未処理の拒否を防ぐ。
    rejectShared(new Error("shared failure"));
    await new Promise((resolve) => setImmediate(resolve));
  });
});
