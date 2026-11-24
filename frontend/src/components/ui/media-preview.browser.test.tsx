import type { ReactNode } from "react";
import {
  render as renderComponent,
  screen,
  fireEvent,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vite-plus/test";
import { page } from "vite-plus/test/browser/context";
import pdfUrl from "../../../.storybook/fixtures/media.pdf?url";
import { MediaPreview } from "./media-preview";
import "../../styles.css";
import { DESKTOP_PROTOCOL_VERSION, resetDesktopForTest } from "../../lib/desktop";
import { resetDesktopDownloadsForTest } from "../../lib/desktopDownloads";
import { afterEach } from "vite-plus/test";

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
    if (cmd === "zoo_session")
      return {
        protocol: DESKTOP_PROTOCOL_VERSION,
        ui: "remote",
        api_base_url: "https://zli.chat",
        state: { state: "ready" },
      };
    if (cmd === "zoo_download_settings") return { ask_every_time: true, directory: null };
    if (cmd === "zoo_download_begin")
      return { status: "started", id: "dl", path: "/tmp/report.txt", prompted: true };
    if (cmd === "zoo_download_finish") return "saved.txt";
    if (cmd === "zoo_download_write" || cmd === "zoo_download_cancel") return undefined;
    throw new Error(`command ${cmd} not found`);
  });
}

import { LocaleProvider } from "../i18n/LocaleProvider";

const render = (element: ReactNode) => renderComponent(<LocaleProvider>{element}</LocaleProvider>);

it.each([390, 1280])(
  "defers PDF loading and supports vertical scroll, page buttons, and page jump at %ipx",
  async (width) => {
    await page.viewport(width, 844);
    const user = userEvent.setup();
    const view = render(
      <div style={{ width: 280 }}>
        <MediaPreview src={pdfUrl} name="Report" contentType="application/pdf" />
      </div>,
    );
    expect(view.container.querySelector("canvas")).toBeNull();
    fireEvent.click(screen.getByRole("button"));
    const dialog = await screen.findByRole("dialog");
    const canvas = dialog.querySelector("canvas")!;
    await waitFor(() => expect(canvas.width).toBeGreaterThan(300), { timeout: 10_000 });
    expect(canvas.getBoundingClientRect().width).toBeGreaterThan(280);

    const scrollRegion = screen.getByRole("region", { name: "PDFページ一覧" });
    const pageInput = screen.getByRole("textbox", { name: "ページ番号" }) as HTMLInputElement;
    const prev = screen.getByRole("button", { name: "前のページ" });
    const next = screen.getByRole("button", { name: "次のページ" });
    await waitFor(() => expect(next).not.toBeDisabled(), { timeout: 10_000 });
    expect(prev).toBeDisabled();
    expect(pageInput.value).toBe("1");
    expect(dialog.textContent).toContain("/ 12");

    // Pages are stacked vertically in continuous order.
    const slot1 = dialog.querySelector('[data-pdf-page="1"]')!;
    const slot2 = dialog.querySelector('[data-pdf-page="2"]')!;
    expect(slot2.getBoundingClientRect().top).toBeGreaterThan(slot1.getBoundingClientRect().bottom);

    // Previous / Next buttons step across pages.
    await user.click(next);
    await waitFor(() => expect(pageInput.value).toBe("2"), { timeout: 10_000 });
    await waitFor(
      () =>
        expect(
          (dialog.querySelector('[data-pdf-page="2"] canvas') as HTMLCanvasElement | null)?.width,
        ).toBeGreaterThan(300),
      { timeout: 10_000 },
    );

    await user.click(prev);
    await waitFor(() => expect(pageInput.value).toBe("1"), { timeout: 10_000 });

    // Vertical scroll switches the current page and renders newly visible pages.
    const slot3 = dialog.querySelector('[data-pdf-page="3"]') as HTMLElement;
    scrollRegion.scrollTop = slot3.offsetTop;
    fireEvent.scroll(scrollRegion);
    await waitFor(() => expect(pageInput.value).toBe("3"), { timeout: 10_000 });
    await waitFor(
      () =>
        expect(
          (dialog.querySelector('[data-pdf-page="3"] canvas') as HTMLCanvasElement | null)?.width,
        ).toBeGreaterThan(300),
      { timeout: 10_000 },
    );

    // Typing a page number and pressing Enter jumps directly and releases far off-screen canvases.
    await user.click(pageInput);
    await user.keyboard("10{Enter}");
    await waitFor(() => expect(pageInput.value).toBe("10"), { timeout: 10_000 });
    await waitFor(
      () =>
        expect(
          (dialog.querySelector('[data-pdf-page="10"] canvas') as HTMLCanvasElement | null)?.width,
        ).toBeGreaterThan(300),
      { timeout: 10_000 },
    );
    expect(dialog.querySelector('[data-pdf-page="1"] canvas')).toBeNull();

    // Out-of-range numbers clamp cleanly and Previous page still steps back from the bottom.
    await user.click(pageInput);
    await user.keyboard("99{Enter}");
    await waitFor(() => expect(pageInput.value).toBe("12"), { timeout: 10_000 });
    expect(next).toBeDisabled();
    await user.click(prev);
    await waitFor(() => expect(pageInput.value).toBe("11"), { timeout: 10_000 });

    // Invalid input reverts on blur, while clicking Prev/Next does not commit draft digits.
    await user.click(pageInput);
    await user.keyboard("abc");
    scrollRegion.focus();
    await waitFor(() => expect(pageInput.value).toBe("11"));

    await user.click(pageInput);
    await user.keyboard("2");
    await user.click(prev);
    await waitFor(() => expect(pageInput.value).toBe("10"), { timeout: 10_000 });

    // Blurring after typing a valid number jumps (needed on touch numeric keypads without Enter).
    await user.click(pageInput);
    await user.keyboard("4");
    scrollRegion.focus();
    await waitFor(() => expect(pageInput.value).toBe("4"), { timeout: 10_000 });
  },
);

it("does not jump while IME composition is active", async () => {
  render(<MediaPreview src={pdfUrl} name="Report" contentType="application/pdf" initialOpen />);
  const pageInput = (await screen.findByRole("textbox", {
    name: "ページ番号",
  })) as HTMLInputElement;
  await waitFor(() => expect(pageInput).not.toBeDisabled(), { timeout: 10_000 });

  fireEvent.focus(pageInput);
  fireEvent.compositionStart(pageInput);
  fireEvent.change(pageInput, { target: { value: "5" } });
  fireEvent.keyDown(pageInput, { key: "Enter", isComposing: true });
  fireEvent.submit(pageInput.closest("form")!);
  expect(pageInput.value).toBe("5");
  expect(screen.getByRole("button", { name: "前のページ" })).toBeDisabled();

  fireEvent.compositionEnd(pageInput);
  // Safari では compositionend が先に来ても keyCode 229 が残る。
  fireEvent.keyDown(pageInput, { key: "Enter", isComposing: false, keyCode: 229 });
  fireEvent.submit(pageInput.closest("form")!);
  expect(pageInput.value).toBe("5");
  expect(screen.getByRole("button", { name: "前のページ" })).toBeDisabled();

  fireEvent.keyDown(pageInput, { key: "Enter", isComposing: false });
  fireEvent.submit(pageInput.closest("form")!);
  await waitFor(
    () => expect(screen.getByRole("button", { name: "前のページ" })).not.toBeDisabled(),
    { timeout: 10_000 },
  );
});

it("does not share save state across same-named previews with unknown src", async () => {
  const base = desktopInvoke();
  let resolveFinish = (_name: string) => {};
  const finishing = new Promise<string>((resolve) => {
    resolveFinish = resolve;
  });
  const invoke = vi.fn((cmd) => (cmd === "zoo_download_finish" ? finishing : base(cmd)));
  mockDesktop(invoke);
  vi.stubGlobal(
    "fetch",
    vi.fn(
      async () => new Response(new Uint8Array([1, 2, 3]), { headers: { "content-length": "3" } }),
    ),
  );

  const { rerender } = render(
    <>
      <MediaPreview name="sample.txt" src={null} contentType="text/plain" />
      <MediaPreview name="sample.txt" src={null} contentType="text/plain" />
    </>,
  );
  rerender(
    <>
      <MediaPreview name="sample.txt" src="https://files.test/a.txt" contentType="text/plain" />
      <MediaPreview name="sample.txt" src="https://files.test/b.txt" contentType="text/plain" />
    </>,
  );

  const openButtons = screen.getAllByRole("button", { name: /sample.txt/ });
  fireEvent.click(openButtons.find(b => b.getAttribute("aria-label")?.includes("プレビュー")) ?? openButtons[0]);
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

it("shares save state when saveId is provided", async () => {
  const base = desktopInvoke();
  let resolveFinish = (_name: string) => {};
  const finishing = new Promise<string>((resolve) => {
    resolveFinish = resolve;
  });
  const invoke = vi.fn((cmd) => (cmd === "zoo_download_finish" ? finishing : base(cmd)));
  mockDesktop(invoke);
  vi.stubGlobal(
    "fetch",
    vi.fn(
      async () => new Response(new Uint8Array([1, 2, 3]), { headers: { "content-length": "3" } }),
    ),
  );

  render(
    <>
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
    </>,
  );

  const openButtons = screen.getAllByRole("button", { name: /sample.txt/ });
  fireEvent.click(openButtons.find(b => b.getAttribute("aria-label")?.includes("プレビュー")) ?? openButtons[0]);
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

it("treats keyCode 229 without a recent composition as a normal Enter", async () => {
  await page.viewport(1280, 844);
  render(<MediaPreview src={pdfUrl} name="Report" contentType="application/pdf" initialOpen />);
  const pageInput = (await screen.findByRole("textbox", {
    name: "ページ番号",
  })) as HTMLInputElement;
  await waitFor(() => expect(pageInput).not.toBeDisabled(), { timeout: 10_000 });

  await userEvent.clear(pageInput);
  await userEvent.type(pageInput, "5");
  fireEvent.keyDown(pageInput, { key: "Enter", isComposing: false, keyCode: 229 });
  fireEvent.submit(pageInput.closest("form")!);
  await waitFor(() => expect(pageInput.value).toBe("5"), { timeout: 10_000 });
  // 入力欄の値だけではジャンプせずにも通るため、実際に5ページへ移動したことを確認する。
  fireEvent.click(screen.getByRole("button", { name: "前のページ" }));
  await waitFor(() => expect(pageInput.value).toBe("4"), { timeout: 10_000 });
});

it("jumps on Enter even with full-width digits and surrounding spaces", async () => {
  await page.viewport(1280, 844);
  render(<MediaPreview src={pdfUrl} name="Report" contentType="application/pdf" initialOpen />);
  const pageInput = (await screen.findByRole("textbox", {
    name: "ページ番号",
  })) as HTMLInputElement;
  await waitFor(() => expect(pageInput).not.toBeDisabled(), { timeout: 10_000 });

  await userEvent.clear(pageInput);
  await userEvent.type(pageInput, " １２ ");
  await userEvent.keyboard("{Enter}");
  await waitFor(() => expect(pageInput.value).toBe("12"), { timeout: 10_000 });
});

it("commits the draft page when tab moves focus to a page button", async () => {
  await page.viewport(1280, 844);
  render(<MediaPreview src={pdfUrl} name="Report" contentType="application/pdf" initialOpen />);
  const pageInput = (await screen.findByRole("textbox", {
    name: "ページ番号",
  })) as HTMLInputElement;
  await waitFor(() => expect(pageInput).not.toBeDisabled(), { timeout: 10_000 });

  await userEvent.clear(pageInput);
  await userEvent.type(pageInput, "5");
  await userEvent.tab();
  await waitFor(() => expect(pageInput.value).toBe("5"), { timeout: 10_000 });
  expect(screen.getByRole("button", { name: "前のページ" })).not.toBeDisabled();
});

it("discards the draft on a touch tap in the same order as pointerup, blur, click", async () => {
  await page.viewport(1280, 844);
  render(<MediaPreview src={pdfUrl} name="Report" contentType="application/pdf" initialOpen />);
  const pageInput = (await screen.findByRole("textbox", {
    name: "ページ番号",
  })) as HTMLInputElement;
  await waitFor(() => expect(pageInput).not.toBeDisabled(), { timeout: 10_000 });

  await userEvent.clear(pageInput);
  await userEvent.type(pageInput, "10{Enter}");
  await waitFor(() => expect(pageInput.value).toBe("10"), { timeout: 10_000 });
  await userEvent.clear(pageInput);
  await userEvent.type(pageInput, "2");

  const prev = screen.getByRole("button", { name: "前のページ" });
  fireEvent.pointerDown(prev);
  fireEvent.pointerUp(window);
  fireEvent.blur(pageInput);
  fireEvent.click(prev);
  await waitFor(() => expect(pageInput.value).toBe("9"), { timeout: 10_000 });
});

it("keeps the draft when a touch scroll cancels the page-button press", async () => {
  await page.viewport(1280, 844);
  render(<MediaPreview src={pdfUrl} name="Report" contentType="application/pdf" initialOpen />);
  const pageInput = (await screen.findByRole("textbox", {
    name: "ページ番号",
  })) as HTMLInputElement;
  await waitFor(() => expect(pageInput).not.toBeDisabled(), { timeout: 10_000 });

  await userEvent.clear(pageInput);
  await userEvent.type(pageInput, "5");
  const next = screen.getByRole("button", { name: "次のページ" });
  fireEvent.pointerDown(next);
  fireEvent.pointerCancel(next);
  fireEvent.blur(pageInput);
  await waitFor(() => expect(pageInput.value).toBe("5"), { timeout: 10_000 });
  expect(screen.getByRole("button", { name: "前のページ" })).not.toBeDisabled();
});

it("resets a stale page-button press when the input regains focus", async () => {
  await page.viewport(1280, 844);
  render(<MediaPreview src={pdfUrl} name="Report" contentType="application/pdf" initialOpen />);
  const pageInput = (await screen.findByRole("textbox", {
    name: "ページ番号",
  })) as HTMLInputElement;
  await waitFor(() => expect(pageInput).not.toBeDisabled(), { timeout: 10_000 });

  fireEvent.pointerDown(screen.getByRole("button", { name: "次のページ" }));
  fireEvent.pointerUp(window);
  await userEvent.click(pageInput);
  await userEvent.type(pageInput, "5");
  fireEvent.blur(pageInput);
  await waitFor(() => expect(pageInput.value).toBe("5"), { timeout: 10_000 });
});

it.each(["video/mp4", "audio/mpeg"])("provides controls without autoplay for %s", (contentType) => {
  const view = render(<MediaPreview src="/media" name="Media" contentType={contentType} />);
  const media = view.container.querySelector("video, audio") as HTMLMediaElement;
  expect(media.controls).toBe(true);
  expect(media.autoplay).toBe(false);
  expect(media.preload).toBe("none");
  fireEvent.error(media);
  expect(screen.getByRole("alert")).toBeTruthy();
});

it("keeps full width in a narrow placement and opens the same image viewer", async () => {
  const view = render(
    <div style={{ width: 280 }}>
      <MediaPreview
        src="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAMAAAABCAYAAAAb4BS0AAAADklEQVR4nGP438DwH4YBMWYHe7X3S+QAAAAASUVORK5CYII="
        name="Photo"
        contentType="image/png"
      />
    </div>,
  );
  expect(view.container.querySelector("button")!.getBoundingClientRect().width).toBeGreaterThan(
    270,
  );
  fireEvent.click(screen.getByRole("button"));
  expect(await screen.findByRole("dialog")).toBeTruthy();
});
