import "../../styles.css";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { toast } from "sonner";
import { afterEach, expect, it, vi } from "vite-plus/test";

import { DESKTOP_PROTOCOL_VERSION, resetDesktopForTest } from "../../lib/desktop";
import { resetDesktopDownloadsForTest } from "../../lib/desktopDownloads";
import { LocaleProvider } from "../i18n/LocaleProvider";
import { MediaPreview } from "../ui/media-preview";
import { PreviewDownloadButton } from "./PreviewDownloadButton";

const SRC = "https://files.test/report.txt";

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  delete (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
  resetDesktopForTest();
  resetDesktopDownloadsForTest();
});

function mockDesktop(invoke: (cmd: string, args?: unknown) => Promise<unknown>) {
  (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {
    invoke,
    transformCallback: vi.fn(),
  };
  resetDesktopForTest();
}

function desktopInvoke() {
  return vi.fn(async (cmd: string) => {
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
      return { status: "started", id: "dl", path: "/tmp/report.txt", prompted: true };
    }
    if (cmd === "zoo_download_finish") return "saved.txt";
    if (cmd === "zoo_download_write") return undefined;
    if (cmd === "zoo_download_cancel") return undefined;
    throw new Error(`command ${cmd} not found`);
  });
}

function renderButton() {
  return render(
    <LocaleProvider>
      <PreviewDownloadButton name="report.txt" src={SRC} showLabel />
    </LocaleProvider>,
  );
}

it("saves a preview body through the desktop download commands", async () => {
  const invoke = desktopInvoke();
  mockDesktop(invoke);
  const fetchMock = vi.fn(
    async () => new Response(new Uint8Array([1, 2, 3]), { headers: { "content-length": "3" } }),
  );
  vi.stubGlobal("fetch", fetchMock);
  const saved = vi.spyOn(toast, "success").mockReturnValue("saved");

  renderButton();
  fireEvent.click(await screen.findByRole("button", { name: "report.txtをダウンロード" }));

  await waitFor(() => expect(invoke).toHaveBeenCalledWith("zoo_download_finish", { id: "dl" }));
  expect(fetchMock).toHaveBeenCalledWith(SRC, expect.objectContaining({ credentials: "omit" }));
  expect(invoke).toHaveBeenCalledWith("zoo_download_begin", {
    begin: { file_name: "report.txt" },
  });
  expect(invoke).toHaveBeenCalledWith(
    "zoo_download_write",
    expect.any(Uint8Array),
    expect.objectContaining({ headers: expect.objectContaining({ "x-zoo-download-id": "dl" }) }),
  );
  expect(saved).toHaveBeenCalledWith("saved.txtを保存しました。");
});

it("switches to the external link when the preview body cannot be fetched", async () => {
  const invoke = desktopInvoke();
  mockDesktop(invoke);
  vi.stubGlobal(
    "fetch",
    vi.fn(async () => {
      throw new TypeError("Failed to fetch");
    }),
  );
  const failed = vi.spyOn(toast, "error").mockReturnValue("failed");

  renderButton();
  fireEvent.click(await screen.findByRole("button", { name: "report.txtをダウンロード" }));

  await waitFor(() =>
    expect(failed).toHaveBeenCalledWith(
      "このファイルはアプリから保存できません。リンクを別のタブで開いて保存してください。",
      undefined,
    ),
  );
  const link = await screen.findByRole("link", { name: "ダウンロード" });
  expect(link.getAttribute("href")).toBe(SRC);
  expect(link.getAttribute("target")).toBe("_blank");
  expect(invoke).not.toHaveBeenCalledWith("zoo_download_finish", { id: "dl" });
});

it("keeps the original link in a browser", () => {
  renderButton();
  const link = screen.getByRole("link", { name: "ダウンロード" });
  expect(link.getAttribute("href")).toBe(SRC);
  expect(link.getAttribute("target")).toBe("_blank");
});

it("saves the body from the default footer of MediaPreview", async () => {
  const invoke = desktopInvoke();
  mockDesktop(invoke);
  vi.stubGlobal(
    "fetch",
    vi.fn(async () => new Response(new Uint8Array([1, 2]))),
  );
  vi.spyOn(toast, "success").mockReturnValue("saved");

  render(
    <LocaleProvider>
      <MediaPreview name="sample.txt" src="data:text/plain,Hello" contentType="text/plain" />
    </LocaleProvider>,
  );
  fireEvent.click(screen.getByRole("button", { name: "sample.txtをプレビュー" }));
  const dialog = await screen.findByRole("dialog");
  fireEvent.click(within(dialog).getByRole("button", { name: "sample.txtをダウンロード" }));

  await waitFor(() => expect(invoke).toHaveBeenCalledWith("zoo_download_finish", { id: "dl" }));
});
