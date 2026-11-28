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
  const cancelCheck = vi.fn(async () => {});
  const checked = new Response(
    new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(new Uint8Array([1, 2, 3]));
        controller.close();
      },
      cancel: cancelCheck,
    }),
    { headers: { "content-length": "3" } },
  );
  const base = desktopInvoke();
  let releaseBegin!: (value: unknown) => void;
  const beginning = new Promise<unknown>((resolve) => (releaseBegin = resolve));
  const invoke = vi.fn((cmd: string) => (cmd === "zoo_download_begin" ? beginning : base(cmd)));
  mockDesktop(invoke);
  const fetchMock = vi
    .fn()
    .mockResolvedValueOnce(checked)
    .mockResolvedValue(new Response(new Uint8Array([1, 2, 3])));
  vi.stubGlobal("fetch", fetchMock);
  const saved = vi.spyOn(toast, "success").mockReturnValue("saved");

  renderButton();
  fireEvent.click(await screen.findByRole("button", { name: "report.txtをダウンロード" }));

  await waitFor(() =>
    expect(invoke).toHaveBeenCalledWith("zoo_download_begin", {
      begin: { file_name: "report.txt", size: 3 },
    }),
  );
  // 確認用の応答はダイアログの前に破棄し、本文は保存時に開き直す。
  expect(cancelCheck).toHaveBeenCalledOnce();
  expect(fetchMock).toHaveBeenCalledTimes(1);
  expect(fetchMock).toHaveBeenCalledWith(SRC, expect.objectContaining({ credentials: "omit" }));

  releaseBegin({ status: "started", id: "dl", path: "/tmp/report.txt", prompted: true });
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("zoo_download_finish", { id: "dl" }));
  expect(fetchMock).toHaveBeenCalledTimes(2);
  expect(invoke).toHaveBeenCalledWith(
    "zoo_download_write",
    expect.any(Uint8Array),
    expect.objectContaining({ headers: expect.objectContaining({ "x-zoo-download-id": "dl" }) }),
  );
  expect(saved).toHaveBeenCalledWith("saved.txtを保存しました。");
});

it("does not pass encoded content-length as save size limit", async () => {
  const base = desktopInvoke();
  let releaseBegin!: (value: unknown) => void;
  const beginning = new Promise<unknown>((resolve) => (releaseBegin = resolve));
  const invoke = vi.fn((cmd: string) => (cmd === "zoo_download_begin" ? beginning : base(cmd)));
  mockDesktop(invoke);
  vi.stubGlobal(
    "fetch",
    vi.fn(
      async () =>
        new Response("", { headers: { "content-length": "12", "content-encoding": "gzip" } }),
    ),
  );
  renderButton();
  fireEvent.click(await screen.findByRole("button", { name: "report.txtをダウンロード" }));
  await waitFor(() =>
    expect(invoke).toHaveBeenCalledWith("zoo_download_begin", {
      begin: { file_name: "report.txt" },
    }),
  );
  releaseBegin({ status: "started", id: "dl", path: "/tmp/report.txt", prompted: true });
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("zoo_download_finish", { id: "dl" }));
});

it("refreshes a signed URL that expires while the save dialog is open", async () => {
  const expired = "https://files.test/expired";
  const fresh = "https://files.test/fresh";
  const base = desktopInvoke();
  let releaseBegin!: (value: unknown) => void;
  const beginning = new Promise<unknown>((resolve) => (releaseBegin = resolve));
  const invoke = vi.fn((cmd: string) => (cmd === "zoo_download_begin" ? beginning : base(cmd)));
  mockDesktop(invoke);
  const resolveUrl = vi.fn(async ({ refresh }: { refresh: boolean }) =>
    refresh ? fresh : expired,
  );
  let expiredCalls = 0;
  const fetchMock = vi.fn(async (url: string) => {
    if (url === expired) {
      expiredCalls += 1;
      // 確認のときだけ有効で、保存ダイアログのあとは期限切れになる。
      return expiredCalls === 1
        ? new Response(new Uint8Array([1, 2, 3]), { headers: { "content-length": "3" } })
        : new Response(null, { status: 403 });
    }
    return new Response(new Uint8Array([1, 2, 3]));
  });
  vi.stubGlobal("fetch", fetchMock);
  const saved = vi.spyOn(toast, "success").mockReturnValue("saved");

  render(
    <LocaleProvider>
      <PreviewDownloadButton name="report.txt" src={expired} showLabel resolveUrl={resolveUrl} />
    </LocaleProvider>,
  );
  fireEvent.click(await screen.findByRole("button", { name: "report.txtをダウンロード" }));

  await waitFor(() =>
    expect(invoke).toHaveBeenCalledWith("zoo_download_begin", {
      begin: { file_name: "report.txt", size: 3 },
    }),
  );
  releaseBegin({ status: "started", id: "dl", path: "/tmp/report.txt", prompted: true });

  await waitFor(() => expect(invoke).toHaveBeenCalledWith("zoo_download_finish", { id: "dl" }), {
    timeout: 3000,
  });
  expect(resolveUrl).toHaveBeenCalledWith({ refresh: false });
  expect(resolveUrl).toHaveBeenCalledWith({ refresh: true });
  expect(fetchMock).toHaveBeenCalledWith(fresh, expect.objectContaining({ credentials: "omit" }));
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
  expect(invoke).not.toHaveBeenCalledWith("zoo_download_begin", expect.anything());
  expect(invoke).not.toHaveBeenCalledWith("zoo_download_finish", { id: "dl" });
});

it("does not replace an expired blob URL with a stale external link", async () => {
  const invoke = desktopInvoke();
  mockDesktop(invoke);
  vi.stubGlobal(
    "fetch",
    vi.fn(async () => {
      throw new TypeError("Failed to fetch");
    }),
  );
  const failed = vi.spyOn(toast, "error").mockReturnValue("failed");

  render(
    <LocaleProvider>
      <PreviewDownloadButton name="report.txt" src="blob:expired" showLabel />
    </LocaleProvider>,
  );
  fireEvent.click(await screen.findByRole("button", { name: "report.txtをダウンロード" }));

  await waitFor(() =>
    expect(failed).toHaveBeenCalledWith("ダウンロードできませんでした。", undefined),
  );
  expect(screen.queryByRole("link")).toBeNull();
  expect(invoke).not.toHaveBeenCalledWith("zoo_download_begin", expect.anything());
});

it("cancels while the preview body is still being opened", async () => {
  const invoke = desktopInvoke();
  mockDesktop(invoke);
  let fetchSignal: AbortSignal | undefined;
  vi.stubGlobal(
    "fetch",
    vi.fn((_url: string, init?: RequestInit) => {
      fetchSignal = init?.signal ?? undefined;
      return new Promise<Response>((_resolve, reject) => {
        fetchSignal?.addEventListener("abort", () =>
          reject(new DOMException("Aborted", "AbortError")),
        );
      });
    }),
  );
  const failed = vi.spyOn(toast, "error").mockReturnValue("failed");

  renderButton();
  fireEvent.click(await screen.findByRole("button", { name: "report.txtをダウンロード" }));
  await waitFor(() => expect(fetchSignal?.aborted).toBe(false));

  fireEvent.click(screen.getByRole("button", { name: "report.txtのダウンロードを中止" }));

  await waitFor(() =>
    expect(screen.getByRole("button", { name: "report.txtをダウンロード" })).toBeEnabled(),
  );
  expect(fetchSignal?.aborted).toBe(true);
  expect(failed).not.toHaveBeenCalled();
  expect(invoke).not.toHaveBeenCalledWith("zoo_download_begin", expect.anything());
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

it("shares one save across the card and enlarged preview", async () => {
  const base = desktopInvoke();
  let resolveFinish!: (name: string) => void;
  const finishing = new Promise<string>((resolve) => {
    resolveFinish = resolve;
  });
  const invoke = vi.fn((cmd: string) => (cmd === "zoo_download_finish" ? finishing : base(cmd)));
  mockDesktop(invoke);
  vi.stubGlobal(
    "fetch",
    vi.fn(async () => new Response(new Uint8Array([1, 2, 3]))),
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

  fireEvent.click(within(dialog).getByRole("button", { name: /閉じる/ }));
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  fireEvent.click(screen.getByRole("button", { name: "sample.txtをプレビュー" }));
  await screen.findByRole("dialog");

  const begins = invoke.mock.calls.filter(([cmd]) => cmd === "zoo_download_begin");
  expect(begins).toHaveLength(1);
  expect(screen.queryByRole("button", { name: "sample.txtをダウンロード" })).toBeNull();

  resolveFinish("saved.txt");
  await waitFor(() =>
    expect(screen.queryByRole("button", { name: "sample.txtをダウンロード" })).not.toBeNull(),
  );
});

it("keeps progress when preview src changes during save", async () => {
  const base = desktopInvoke();
  let resolveFinish!: (name: string) => void;
  const finishing = new Promise<string>((resolve) => {
    resolveFinish = resolve;
  });
  const invoke = vi.fn((cmd: string) => (cmd === "zoo_download_finish" ? finishing : base(cmd)));
  mockDesktop(invoke);
  vi.stubGlobal(
    "fetch",
    vi.fn(
      async () => new Response(new Uint8Array([1, 2, 3]), { headers: { "content-length": "3" } }),
    ),
  );
  vi.spyOn(toast, "success").mockReturnValue("saved");

  const { rerender } = render(
    <LocaleProvider>
      <MediaPreview name="sample.txt" src="https://files.test/a.txt" contentType="text/plain" />
    </LocaleProvider>,
  );
  fireEvent.click(screen.getByRole("button", { name: "sample.txtをプレビュー" }));
  const dialog = await screen.findByRole("dialog");
  fireEvent.click(within(dialog).getByRole("button", { name: "sample.txtをダウンロード" }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("zoo_download_finish", { id: "dl" }));

  rerender(
    <LocaleProvider>
      <MediaPreview name="sample.txt" src="https://files.test/b.txt" contentType="text/plain" />
    </LocaleProvider>,
  );

  // 押した側だけでなく、同じプレビューの別ボタン（カード側）も同じ保存状態に追従する。
  fireEvent.click(within(dialog).getByRole("button", { name: /閉じる/ }));
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  expect(screen.queryByRole("button", { name: "sample.txtをダウンロード" })).toBeNull();
  expect(screen.getByText("保存しています…")).toBeInTheDocument();
  expect(invoke.mock.calls.filter(([cmd]) => cmd === "zoo_download_begin")).toHaveLength(1);

  resolveFinish("saved.txt");
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "sample.txtをダウンロード" })).not.toBeNull(),
  );
});

it("restores progress after remounting the same preview", async () => {
  const base = desktopInvoke();
  let resolveFinish!: (name: string) => void;
  const finishing = new Promise<string>((resolve) => {
    resolveFinish = resolve;
  });
  const invoke = vi.fn((cmd: string) => (cmd === "zoo_download_finish" ? finishing : base(cmd)));
  mockDesktop(invoke);
  vi.stubGlobal(
    "fetch",
    vi.fn(
      async () => new Response(new Uint8Array([1, 2, 3]), { headers: { "content-length": "3" } }),
    ),
  );
  vi.spyOn(toast, "success").mockReturnValue("saved");

  const { unmount } = render(
    <LocaleProvider>
      <MediaPreview name="sample.txt" src="https://files.test/a.txt" contentType="text/plain" />
    </LocaleProvider>,
  );
  fireEvent.click(screen.getByRole("button", { name: "sample.txtをプレビュー" }));
  let dialog = await screen.findByRole("dialog");
  fireEvent.click(within(dialog).getByRole("button", { name: "sample.txtをダウンロード" }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("zoo_download_finish", { id: "dl" }));

  unmount();
  render(
    <LocaleProvider>
      <MediaPreview name="sample.txt" src="https://files.test/a.txt" contentType="text/plain" />
    </LocaleProvider>,
  );
  fireEvent.click(screen.getByRole("button", { name: "sample.txtをプレビュー" }));
  dialog = await screen.findByRole("dialog");
  expect(within(dialog).queryByRole("button", { name: "sample.txtをダウンロード" })).toBeNull();
  expect(invoke.mock.calls.filter(([cmd]) => cmd === "zoo_download_begin")).toHaveLength(1);

  resolveFinish("saved.txt");
  await waitFor(() =>
    expect(within(dialog).getByRole("button", { name: "sample.txtをダウンロード" })).not.toBeNull(),
  );
});

it("does not share save state across same-named previews with unknown src", async () => {
  const base = desktopInvoke();
  let resolveFinish!: (name: string) => void;
  const finishing = new Promise<string>((resolve) => {
    resolveFinish = resolve;
  });
  const invoke = vi.fn((cmd: string) => (cmd === "zoo_download_finish" ? finishing : base(cmd)));
  mockDesktop(invoke);
  vi.stubGlobal(
    "fetch",
    vi.fn(
      async () => new Response(new Uint8Array([1, 2, 3]), { headers: { "content-length": "3" } }),
    ),
  );

  const { rerender } = render(
    <LocaleProvider>
      <div>
        <MediaPreview name="sample.txt" src={null} contentType="text/plain" />
        <MediaPreview name="sample.txt" src={null} contentType="text/plain" />
      </div>
    </LocaleProvider>,
  );
  rerender(
    <LocaleProvider>
      <div>
        <MediaPreview name="sample.txt" src="https://files.test/a.txt" contentType="text/plain" />
        <MediaPreview name="sample.txt" src="https://files.test/b.txt" contentType="text/plain" />
      </div>
    </LocaleProvider>,
  );

  const openButtons = screen.getAllByRole("button", { name: "sample.txtをプレビュー" });
  fireEvent.click(openButtons[0]);
  let dialog = await screen.findByRole("dialog");
  fireEvent.click(within(dialog).getByRole("button", { name: "sample.txtをダウンロード" }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("zoo_download_finish", { id: "dl" }));
  fireEvent.click(within(dialog).getByRole("button", { name: /閉じる/ }));
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());

  fireEvent.click(openButtons[1]);
  dialog = await screen.findByRole("dialog");
  expect(within(dialog).getByRole("button", { name: "sample.txtをダウンロード" })).not.toBeNull();
  expect(invoke.mock.calls.filter(([cmd]) => cmd === "zoo_download_begin")).toHaveLength(1);
  resolveFinish("saved.txt");
});

it("shares save state when saveId is provided to MediaPreview", async () => {
  const base = desktopInvoke();
  let resolveFinish!: (name: string) => void;
  const finishing = new Promise<string>((resolve) => {
    resolveFinish = resolve;
  });
  const invoke = vi.fn((cmd: string) => (cmd === "zoo_download_finish" ? finishing : base(cmd)));
  mockDesktop(invoke);
  vi.stubGlobal(
    "fetch",
    vi.fn(
      async () => new Response(new Uint8Array([1, 2, 3]), { headers: { "content-length": "3" } }),
    ),
  );

  render(
    <LocaleProvider>
      <div>
        <MediaPreview
          name="sample.txt"
          src="https://files.test/a.txt"
          contentType="text/plain"
          saveId="shared:sample"
        />
        <MediaPreview
          name="sample.txt"
          src="https://files.test/b.txt"
          contentType="text/plain"
          saveId="shared:sample"
        />
      </div>
    </LocaleProvider>,
  );

  const openButtons = screen.getAllByRole("button", { name: "sample.txtをプレビュー" });
  fireEvent.click(openButtons[0]);
  let dialog = await screen.findByRole("dialog");
  fireEvent.click(within(dialog).getByRole("button", { name: "sample.txtをダウンロード" }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("zoo_download_finish", { id: "dl" }));
  fireEvent.click(within(dialog).getByRole("button", { name: /閉じる/ }));
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());

  fireEvent.click(openButtons[1]);
  dialog = await screen.findByRole("dialog");
  expect(within(dialog).queryByRole("button", { name: "sample.txtをダウンロード" })).toBeNull();
  expect(invoke.mock.calls.filter(([cmd]) => cmd === "zoo_download_begin")).toHaveLength(1);
  resolveFinish("saved.txt");
});
