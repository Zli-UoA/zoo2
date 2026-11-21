import "../../../styles.css";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { toast } from "sonner";
import { afterEach, expect, it, vi } from "vite-plus/test";
import { AttachmentDownloadError } from "../../../lib/api/attachments";
import { DESKTOP_PROTOCOL_VERSION, resetDesktopForTest } from "../../../lib/desktop";
import { resetDesktopDownloadsForTest } from "../../../lib/desktopDownloads";
import { LocaleProvider } from "../../i18n/LocaleProvider";
import { MessageAttachments } from "./MessageAttachments";
import type { AttachmentUrlState } from "../../../hooks/useAttachments";

const link = vi.hoisted(() => ({ state: {} as AttachmentUrlState, load: vi.fn() }));
vi.mock("../../../hooks/useAttachments", () => ({
  useAttachmentUrl: () => {
    link.load();
    return link.state;
  },
}));

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  delete (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
  resetDesktopForTest();
  resetDesktopDownloadsForTest();
});

function textAttachment() {
  return {
    id: "notes",
    message_id: "message",
    uploader_id: "alice",
    channel_id: "channel",
    file_name: "notes.txt",
    file_size: 12,
    content_type: "text/plain",
    storage_key: "notes",
    created_at: "2026-11-02 00:00:00.000",
  };
}

function renderDownload() {
  return render(
    <LocaleProvider>
      <MessageAttachments attachments={[textAttachment()]} />
    </LocaleProvider>,
  );
}

it.each([160, 640])(
  "keeps attachment geometry through signed URL loading, wide image decode and failure at %ipx",
  async (width) => {
    link.state = {
      url: null,
      status: "loading",
      expiresAt: null,
      reload: vi.fn(),
      getFreshUrl: vi.fn(),
    };
    const attachment = {
      id: "file",
      message_id: "message",
      uploader_id: "alice",
      channel_id: "channel",
      file_name: "wide.png",
      width: 3,
      height: 1,
      blurhash: "LEHV6nWB2yk8pyo0adR*.7kCMdnj",
      file_size: 100,
      content_type: "image/png",
      storage_key: "file",
      created_at: "2026-11-02 00:00:00.000",
    };
    const element = () => (
      <LocaleProvider>
        <div style={{ width }}>
          <MessageAttachments attachments={[attachment]} />
          <div data-testid="following">Next message</div>
        </div>
      </LocaleProvider>
    );
    const view = render(element());
    const top = screen.getByTestId("following").getBoundingClientRect().top;
    link.state = {
      ...link.state,
      status: "ready",
      url: "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAMAAAABCAYAAAAb4BS0AAAADklEQVR4nGP438DwH4YBMWYHe7X3S+QAAAAASUVORK5CYII=",
    };
    view.rerender(element());
    const image = screen.getByRole("img", { name: "wide.png" }) as HTMLImageElement;
    await waitFor(() => expect(image.naturalWidth).toBe(3));
    expect(screen.getByTestId("following").getBoundingClientRect().top).toBe(top);
    link.state = { ...link.state, status: "error" };
    view.rerender(element());
    expect(screen.getByTestId("following").getBoundingClientRect().top).toBe(top);
  },
);

it("shows BlurHash without requesting the original image until a sensitive image is tapped", async () => {
  link.load.mockClear();
  link.state = {
    url: "/sensitive.png",
    status: "ready",
    expiresAt: null,
    reload: vi.fn(),
    getFreshUrl: vi.fn(),
  };
  const attachment = {
    id: "sensitive",
    width: 640,
    height: 480,
    blurhash: "LEHV6nWB2yk8pyo0adR*.7kCMdnj",
    file_name: "private.png",
    file_size: 100,
    content_type: "image/png",
    is_sensitive: true,
  };
  render(
    <LocaleProvider>
      <MessageAttachments attachments={[attachment]} />
    </LocaleProvider>,
  );
  expect(link.load).not.toHaveBeenCalled();
  expect(screen.queryByRole("img")).toBeNull();
  const preview = screen.getByRole("button", { name: "画像を表示" }).querySelector("canvas")!;
  await waitFor(() => expect(preview.getContext("2d")!.getImageData(0, 0, 1, 1).data[3]).toBe(255));
  fireEvent.click(screen.getByRole("button", { name: "画像を表示" }));
  expect(await screen.findByRole("img", { name: "private.png" })).toBeTruthy();
  expect(link.load).toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "画像を隠す" }));
  expect(screen.queryByRole("img")).toBeNull();
});

it("reports a /url failure with its reason and re-enables the download button", async () => {
  const resolve = vi.fn().mockRejectedValue(new AttachmentDownloadError("url", { status: 503 }));
  link.state = {
    url: null,
    status: "ready",
    expiresAt: null,
    reload: vi.fn(),
    getFreshUrl: resolve as AttachmentUrlState["getFreshUrl"],
  };
  const toastError = vi.spyOn(toast, "error").mockReturnValue("download-error");
  const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
  renderDownload();
  const button = screen.getByRole("button", { name: "notes.txtをダウンロード" });
  fireEvent.click(button);
  expect(button).toBeDisabled();
  await waitFor(() => expect(toastError).toHaveBeenCalled(), { timeout: 5_000 });
  // `/url` の失敗は、署名付き URL の HTTP 詳細（description）の対象外。
  expect(toastError).toHaveBeenCalledWith("ダウンロードできませんでした。", undefined);
  expect(consoleError).toHaveBeenCalledWith(
    "添付のダウンロードに失敗しました",
    expect.objectContaining({ attachmentId: "notes", reason: "url", status: 503 }),
  );
  // 1回目と、URL を取り直した再試行の2回。
  expect(resolve).toHaveBeenCalledTimes(2);
  expect(button).not.toBeDisabled();
});

it("shows the HTTP status when the signed URL returns an error", async () => {
  link.state = {
    url: null,
    status: "ready",
    expiresAt: null,
    reload: vi.fn(),
    getFreshUrl: vi.fn().mockResolvedValue("https://files.test/notes.txt"),
  };
  const fetchMock = vi.fn(async () => new Response(null, { status: 403 }));
  vi.stubGlobal("fetch", fetchMock);
  const toastError = vi.spyOn(toast, "error").mockReturnValue("download-error");
  const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
  renderDownload();
  fireEvent.click(screen.getByRole("button", { name: "notes.txtをダウンロード" }));
  await waitFor(() => expect(toastError).toHaveBeenCalled(), { timeout: 5_000 });
  expect(toastError).toHaveBeenCalledWith("ダウンロードできませんでした。", {
    description: "HTTP 403",
  });
  expect(consoleError).toHaveBeenCalledWith(
    "添付のダウンロードに失敗しました",
    expect.objectContaining({ attachmentId: "notes", reason: "http", status: 403 }),
  );
  // 403 の応答は再試行でも同じなので、2回取得する。
  expect(fetchMock).toHaveBeenCalledTimes(2);
});

it("asks for another folder when the save destination cannot save without overwriting", async () => {
  (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {
    invoke: vi.fn(async (cmd: string) => {
      if (cmd === "zoo_session") {
        return {
          protocol: DESKTOP_PROTOCOL_VERSION,
          ui: "remote",
          api_base_url: "https://zli.chat",
          state: { state: "ready" },
        };
      }
      if (cmd === "zoo_download_settings") return { ask_every_time: true, directory: null };
      if (cmd === "zoo_download_begin") {
        throw { code: "unsupported_destination", message: "この保存先には保存できません" };
      }
      throw new Error(`command ${cmd} not found`);
    }),
    transformCallback: vi.fn(),
  };
  resetDesktopForTest();
  link.state = {
    url: "https://files.test/notes.txt",
    status: "ready",
    expiresAt: null,
    reload: vi.fn(),
    getFreshUrl: vi.fn(),
  };
  const toastError = vi.spyOn(toast, "error").mockReturnValue("download-error");
  vi.spyOn(console, "error").mockImplementation(() => {});
  renderDownload();
  const button = screen.getByRole("button", { name: "notes.txtをダウンロード" });
  fireEvent.click(button);
  await waitFor(() => expect(toastError).toHaveBeenCalled(), { timeout: 5_000 });
  expect(toastError).toHaveBeenCalledWith(
    "この保存先には保存できません。別のフォルダーを選ぶか、設定の「ダウンロード」で保存先を変えてください。",
  );
  expect(button).not.toBeDisabled();
});
