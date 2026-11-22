import { afterEach, describe, expect, it, vi } from "vite-plus/test";

import { AttachmentDownloadError } from "./api/attachments";

const desktop = vi.hoisted(() => ({
  detect: vi.fn(),
  detectionFailure: vi.fn<() => unknown>(() => null),
  hasBridge: vi.fn(() => true),
  settings: vi.fn(),
  probe: vi.fn(),
  isMissing: vi.fn(
    (error: unknown) =>
      /コマンドがありません/.test(
        String((error as { message?: unknown } | null)?.message ?? error),
      ) ||
      /\bcommand\b[^\n]*\bnot found\b/i.test(
        String((error as { message?: unknown } | null)?.message ?? error),
      ) ||
      /\bcommand\b[^\n]*\bnot allowed\b/i.test(
        String((error as { message?: unknown } | null)?.message ?? error),
      ),
  ),
  begin: vi.fn(),
  write: vi.fn(),
  finish: vi.fn(),
  cancel: vi.fn(),
}));

vi.mock("./desktop", () => ({
  detectDesktop: desktop.detect,
  desktopDetectionFailure: desktop.detectionFailure,
  hasDesktopBridge: desktop.hasBridge,
  desktopDownloadSettings: desktop.settings,
  probeDesktopDownloadSettings: desktop.probe,
  isMissingCommandError: desktop.isMissing,
  desktopDownloadBegin: desktop.begin,
  desktopDownloadWrite: desktop.write,
  desktopDownloadFinish: desktop.finish,
  desktopDownloadCancel: desktop.cancel,
}));

import {
  DesktopDownloadCancelledError,
  cancelDesktopDownload,
  desktopDownloadState,
  desktopDownloadsAvailable,
  downloadBlobToDesktop,
  downloadUrlToDesktop,
  resetDesktopDownloadsForTest,
  subscribeDesktopDownloads,
} from "./desktopDownloads";

function started(id = "d1") {
  return { status: "started" as const, id, path: `C:\\Downloads\\${id}` };
}

function blobResponse(bytes: number[], contentType = "text/plain") {
  return new Response(new Uint8Array(bytes), {
    headers: { "content-length": String(bytes.length), "content-type": contentType },
  });
}

function brokenBodyResponse() {
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      controller.error(new Error("一時的な読み取り失敗"));
    },
  });
  return new Response(body, { headers: { "content-length": "1" } });
}

afterEach(() => {
  resetDesktopDownloadsForTest();
  vi.unstubAllGlobals();
  vi.clearAllMocks();
  desktop.hasBridge.mockReturnValue(true);
  desktop.detectionFailure.mockReturnValue(null);
});

describe("desktopDownloadsAvailable", () => {
  it("is true only when the app has the download settings command", async () => {
    desktop.detect.mockResolvedValue(true);
    desktop.probe.mockResolvedValue({ ask_every_time: true, directory: null });
    await expect(desktopDownloadsAvailable()).resolves.toBe(true);
  });

  it("is false in a browser", async () => {
    desktop.hasBridge.mockReturnValue(false);
    await expect(desktopDownloadsAvailable()).resolves.toBe(false);
    expect(desktop.detect).not.toHaveBeenCalled();
  });

  it("stays false when the command is missing from an older app", async () => {
    desktop.detect.mockResolvedValue(true);
    desktop.probe.mockRejectedValue("Command zoo_download_settings not found");
    await expect(desktopDownloadsAvailable()).resolves.toBe(false);
    desktop.probe.mockResolvedValue({ ask_every_time: true, directory: null });
    await expect(desktopDownloadsAvailable()).resolves.toBe(false);
    expect(desktop.probe).toHaveBeenCalledTimes(1);
  });

  it("retries after a temporary failure instead of falling back for the whole session", async () => {
    desktop.detect.mockResolvedValue(true);
    desktop.probe.mockRejectedValueOnce(new Error("設定を読めませんでした"));
    await expect(desktopDownloadsAvailable()).rejects.toThrow("設定を読めませんでした");
    desktop.probe.mockResolvedValueOnce({ ask_every_time: true, directory: null });
    await expect(desktopDownloadsAvailable()).resolves.toBe(true);
    expect(desktop.probe).toHaveBeenCalledTimes(2);
  });

  it("does not fall back to the browser save when the session check fails temporarily", async () => {
    desktop.detect.mockResolvedValue(false);
    desktop.detectionFailure.mockReturnValue(new Error("通信に失敗しました"));
    await expect(desktopDownloadsAvailable()).rejects.toThrow("通信に失敗しました");

    // 次に呼ばれたときは、もう一度セッションを確かめる。
    desktop.detectionFailure.mockReturnValue(null);
    desktop.detect.mockResolvedValue(true);
    desktop.probe.mockResolvedValue({ ask_every_time: true, directory: null });
    await expect(desktopDownloadsAvailable()).resolves.toBe(true);
  });

  it("treats a protocol mismatch as an older app", async () => {
    desktop.detect.mockResolvedValue(false);
    await expect(desktopDownloadsAvailable()).resolves.toBe(false);
    expect(desktop.probe).not.toHaveBeenCalled();
  });
});

describe("downloadUrlToDesktop", () => {
  it("streams the response into the app and reports progress", async () => {
    desktop.detect.mockResolvedValue(true);
    desktop.settings.mockResolvedValue({ ask_every_time: true, directory: null });
    desktop.begin.mockResolvedValue(started());
    desktop.write.mockResolvedValue(undefined);
    desktop.finish.mockResolvedValue(undefined);
    const fetchMock = vi.fn(async () => blobResponse([1, 2, 3, 4, 5, 6]));
    vi.stubGlobal("fetch", fetchMock);
    const resolve = vi.fn().mockResolvedValue("https://files.test/a");
    const fractions: Array<number | null> = [];
    const unsubscribe = subscribeDesktopDownloads(() => {
      const state = desktopDownloadState("a1");
      if (state?.status === "downloading") fractions.push(state.fraction);
    });

    await downloadUrlToDesktop("a1", "a.txt", resolve);

    unsubscribe();
    expect(resolve).toHaveBeenCalledWith({ refresh: false });
    expect(desktop.begin).toHaveBeenCalledWith("a.txt");
    const written = desktop.write.mock.calls.flatMap(([, bytes]) => [...(bytes as Uint8Array)]);
    expect(written).toEqual([1, 2, 3, 4, 5, 6]);
    expect(desktop.finish).toHaveBeenCalledWith("d1");
    expect(desktopDownloadState("a1")).toBeUndefined();
    expect(fractions.at(-1)).toBe(1);
    expect(Math.max(...fractions.filter((value): value is number => value !== null))).toBe(1);
  });

  it("passes the attachment size to the app so the app can set the limit", async () => {
    desktop.begin.mockResolvedValue(started());
    desktop.write.mockResolvedValue(undefined);
    desktop.finish.mockResolvedValue(undefined);
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => blobResponse([1])),
    );

    await downloadUrlToDesktop("a1b", "a.txt", vi.fn().mockResolvedValue("u"), { size: 1234 });

    expect(desktop.begin).toHaveBeenCalledWith("a.txt", 1234);
  });

  it("refreshes the URL and retries once when the first response fails", async () => {
    desktop.begin.mockResolvedValue(started());
    desktop.write.mockResolvedValue(undefined);
    desktop.finish.mockResolvedValue(undefined);
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(new Response(null, { status: 503 }))
      .mockResolvedValueOnce(blobResponse([42]));
    vi.stubGlobal("fetch", fetchMock);
    const resolve = vi
      .fn()
      .mockResolvedValueOnce("https://files.test/expired")
      .mockResolvedValueOnce("https://files.test/fresh");

    await downloadUrlToDesktop("a2", "a.txt", resolve, { retryDelayMs: 0 });

    expect(resolve.mock.calls).toEqual([[{ refresh: false }], [{ refresh: true }]]);
    expect(desktop.write).toHaveBeenCalledTimes(1);
    expect(desktop.finish).toHaveBeenCalledWith("d1");
  });

  it("discards the partial file and retries once when reading the body fails", async () => {
    desktop.begin
      .mockResolvedValueOnce(started("d-body-1"))
      .mockResolvedValueOnce(started("d-body-2"));
    desktop.write.mockResolvedValue(undefined);
    desktop.finish.mockResolvedValue(undefined);
    desktop.cancel.mockResolvedValue(undefined);
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(brokenBodyResponse())
      .mockResolvedValueOnce(blobResponse([42]));
    vi.stubGlobal("fetch", fetchMock);
    const resolve = vi.fn().mockResolvedValue("https://files.test/a");

    await downloadUrlToDesktop("a2b", "a.txt", resolve, { retryDelayMs: 0 });

    expect(desktop.cancel).toHaveBeenCalledWith("d-body-1");
    expect(desktop.begin).toHaveBeenCalledTimes(2);
    expect(desktop.write).toHaveBeenCalledTimes(1);
    expect(desktop.finish).toHaveBeenCalledWith("d-body-2");
    expect(desktopDownloadState("a2b")).toBeUndefined();
  });

  it("does not retry a write failure with a new save", async () => {
    desktop.begin.mockResolvedValue(started());
    const writeFailure = new Error("IPC への書き込みに失敗しました");
    desktop.write.mockRejectedValueOnce(writeFailure);
    desktop.cancel.mockResolvedValue(undefined);
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => blobResponse([1, 2, 3])),
    );
    const resolve = vi.fn().mockResolvedValue("https://files.test/a");

    await expect(downloadUrlToDesktop("a2c", "a.txt", resolve, { retryDelayMs: 0 })).rejects.toBe(
      writeFailure,
    );

    // 書き込みの失敗は同じ結果になりやすいので、本文の取得からやり直さない。
    expect(desktop.begin).toHaveBeenCalledTimes(1);
    expect(resolve).toHaveBeenCalledTimes(1);
    expect(desktop.cancel).toHaveBeenCalledWith("d1");
    expect(desktop.finish).not.toHaveBeenCalled();
  });

  it("does not save an empty or failed body as a file", async () => {
    desktop.begin.mockResolvedValue(started());
    desktop.write.mockResolvedValue(undefined);
    desktop.cancel.mockResolvedValue(undefined);
    const fetchMock = vi.fn(async () => new Response(null, { status: 403 }));
    vi.stubGlobal("fetch", fetchMock);
    const resolve = vi.fn().mockResolvedValue("https://files.test/a");

    const failure = downloadUrlToDesktop("a3", "a.txt", resolve, { retryDelayMs: 0 });
    await expect(failure).rejects.toBeInstanceOf(AttachmentDownloadError);
    await expect(failure).rejects.toMatchObject({ reason: "http", status: 403 });
    expect(desktop.cancel).toHaveBeenCalledWith("d1");
    expect(desktop.finish).not.toHaveBeenCalled();
    // 応答を取れなかっただけなので、本文転送の再試行で保存 ID を増やさない。
    expect(desktop.begin).toHaveBeenCalledTimes(1);
    expect(resolve).toHaveBeenCalledTimes(2);
    expect(desktopDownloadState("a3")).toBeUndefined();
  });

  it("cancels the app's file when the dialog is cancelled after the click", async () => {
    let release!: (value: ReturnType<typeof started>) => void;
    desktop.begin.mockReturnValue(new Promise((resolve) => (release = resolve)));
    desktop.cancel.mockResolvedValue(undefined);
    const promise = downloadUrlToDesktop("a4", "a.txt", vi.fn().mockResolvedValue("u"));
    cancelDesktopDownload("a4");
    release(started("d4"));

    await expect(promise).rejects.toBeInstanceOf(DesktopDownloadCancelledError);
    expect(desktop.cancel).toHaveBeenCalledWith("d4");
    expect(desktopDownloadState("a4")).toBeUndefined();
  });

  it("cancels while the shared URL request is still pending", async () => {
    desktop.begin.mockResolvedValue(started("d4u"));
    desktop.cancel.mockResolvedValue(undefined);
    let releaseUrl!: (url: string) => void;
    const pendingUrl = new Promise<string>((resolveUrl) => (releaseUrl = resolveUrl));
    const resolve = vi.fn().mockReturnValue(pendingUrl);
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);

    const promise = downloadUrlToDesktop("a4u", "a.txt", resolve);
    await vi.waitFor(() => expect(resolve).toHaveBeenCalled());
    cancelDesktopDownload("a4u");

    await expect(promise).rejects.toBeInstanceOf(DesktopDownloadCancelledError);
    expect(desktop.cancel).toHaveBeenCalledWith("d4u");
    expect(fetchMock).not.toHaveBeenCalled();
    expect(desktopDownloadState("a4u")).toBeUndefined();

    // URL の取得自体は共有の要求なので取り消さず、最後まで続けられる。
    releaseUrl("https://files.test/a");
    await expect(pendingUrl).resolves.toBe("https://files.test/a");
  });

  it("cancels the app's file when the stream is stopped halfway", async () => {
    desktop.begin.mockResolvedValue(started("d5"));
    desktop.cancel.mockResolvedValue(undefined);
    const stream = new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(new Uint8Array([1]));
        controller.enqueue(new Uint8Array([2]));
      },
    });
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response(stream)),
    );
    desktop.write.mockImplementation(async () => {
      // 1 つ目のチャンクを書いたところで、利用者が円を押した。
      cancelDesktopDownload("a5");
    });

    const promise = downloadUrlToDesktop("a5", "a.txt", vi.fn().mockResolvedValue("u"));
    await expect(promise).rejects.toBeInstanceOf(DesktopDownloadCancelledError);
    expect(desktop.cancel).toHaveBeenCalledWith("d5");
    expect(desktop.finish).not.toHaveBeenCalled();
    expect(desktopDownloadState("a5")).toBeUndefined();
  });

  it("finishes at 100% even when the response has no content-length", async () => {
    desktop.begin.mockResolvedValue(started("d6"));
    desktop.write.mockResolvedValue(undefined);
    desktop.finish.mockResolvedValue(undefined);
    const stream = new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(new Uint8Array([1, 2]));
        controller.close();
      },
    });
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response(stream)),
    );
    const fractions: Array<number | null> = [];
    const unsubscribe = subscribeDesktopDownloads(() => {
      const state = desktopDownloadState("a6");
      if (state?.status === "downloading") fractions.push(state.fraction);
    });

    await downloadUrlToDesktop("a6", "a.txt", vi.fn().mockResolvedValue("u"));

    unsubscribe();
    expect(fractions).toContain(null);
    expect(fractions.at(-1)).toBe(1);
    expect(desktop.finish).toHaveBeenCalledWith("d6");
  });

  it("ignores a second start for the same attachment and keeps the state across remounts", async () => {
    let release!: (value: ReturnType<typeof started>) => void;
    desktop.begin.mockReturnValue(new Promise((resolve) => (release = resolve)));
    desktop.write.mockResolvedValue(undefined);
    desktop.finish.mockResolvedValue(undefined);
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => blobResponse([1])),
    );

    const first = downloadUrlToDesktop("a7", "a.txt", vi.fn().mockResolvedValue("u"));
    // 行が仮想スクロールで外れて戻ってきても、状態はモジュール側に残っている。
    expect(desktopDownloadState("a7")).toEqual({ status: "preparing" });
    await downloadUrlToDesktop("a7", "a.txt", vi.fn().mockResolvedValue("u"));
    expect(desktop.begin).toHaveBeenCalledTimes(1);

    release(started("d7"));
    await first;
    expect(desktop.finish).toHaveBeenCalledWith("d7");
    expect(desktopDownloadState("a7")).toBeUndefined();
  });

  it("does not abort while the file is being moved into place", async () => {
    desktop.begin.mockResolvedValue(started("d8"));
    desktop.write.mockResolvedValue(undefined);
    let release!: () => void;
    desktop.finish.mockReturnValue(new Promise<void>((resolve) => (release = resolve)));
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => blobResponse([1])),
    );

    const promise = downloadUrlToDesktop("a8", "a.txt", vi.fn().mockResolvedValue("u"));
    await vi.waitFor(() => expect(desktopDownloadState("a8")?.status).toBe("finishing"));
    cancelDesktopDownload("a8");
    release();

    await expect(promise).resolves.toBeUndefined();
    expect(desktop.cancel).not.toHaveBeenCalled();
  });
});

describe("downloadBlobToDesktop", () => {
  it("writes a decrypted blob and reaches a complete circle", async () => {
    desktop.begin.mockResolvedValue(started());
    desktop.write.mockResolvedValue(undefined);
    desktop.finish.mockResolvedValue(undefined);
    const fractions: Array<number | null> = [];
    const unsubscribe = subscribeDesktopDownloads(() => {
      const state = desktopDownloadState("e1");
      if (state?.status === "downloading") fractions.push(state.fraction);
    });

    await downloadBlobToDesktop("e1", "secret.bin", async (_signal, report) => {
      report(1);
      return new Blob([new Uint8Array([9, 8, 7])]);
    });

    unsubscribe();
    const written = desktop.write.mock.calls.flatMap(([, bytes]) => [...(bytes as Uint8Array)]);
    expect(written).toEqual([9, 8, 7]);
    expect(desktop.finish).toHaveBeenCalledWith("d1");
    expect(fractions.at(-1)).toBe(1);
  });

  it("stops a blob stream and cancels the app's file when aborted halfway", async () => {
    desktop.begin.mockResolvedValue(started("e2"));
    desktop.cancel.mockResolvedValue(undefined);
    desktop.write.mockImplementation(async () => {
      cancelDesktopDownload("e2");
    });
    const blob = new Blob([new Uint8Array([1]), new Uint8Array([2])]);

    const promise = downloadBlobToDesktop("e2", "secret.bin", async () => blob);
    await expect(promise).rejects.toBeInstanceOf(DesktopDownloadCancelledError);
    expect(desktop.cancel).toHaveBeenCalledWith("e2");
    expect(desktop.finish).not.toHaveBeenCalled();
    expect(desktopDownloadState("e2")).toBeUndefined();
  });
});
