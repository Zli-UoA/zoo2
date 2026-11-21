import { afterEach, describe, expect, it, vi } from "vite-plus/test";

import { AttachmentDownloadError } from "./api/attachments";

const desktop = vi.hoisted(() => ({
  detect: vi.fn(),
  settings: vi.fn(),
  begin: vi.fn(),
  write: vi.fn(),
  finish: vi.fn(),
  cancel: vi.fn(),
}));

vi.mock("./desktop", () => ({
  detectDesktop: desktop.detect,
  desktopDownloadSettings: desktop.settings,
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

afterEach(() => {
  resetDesktopDownloadsForTest();
  vi.unstubAllGlobals();
  vi.clearAllMocks();
});

describe("desktopDownloadsAvailable", () => {
  it("is true only when the app has the download settings command", async () => {
    desktop.detect.mockResolvedValue(true);
    desktop.settings.mockResolvedValue({ ask_every_time: true, directory: null });
    await expect(desktopDownloadsAvailable()).resolves.toBe(true);
  });

  it("is false in a browser or an older app", async () => {
    desktop.detect.mockResolvedValue(false);
    await expect(desktopDownloadsAvailable()).resolves.toBe(false);
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
});
