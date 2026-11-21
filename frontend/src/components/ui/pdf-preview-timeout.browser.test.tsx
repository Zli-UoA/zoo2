import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vite-plus/test";
import { PdfPreview } from "./pdf-preview";
import { LocaleProvider } from "../i18n/LocaleProvider";

const mocked = vi.hoisted(() => ({
  getDocument: vi.fn(),
  destroy: vi.fn(async () => {}),
  getPage: vi.fn(),
  fetch: vi.fn(),
}));
vi.mock("pdfjs-dist/legacy/build/pdf.mjs", () => ({
  GlobalWorkerOptions: {},
  version: "test",
  getDocument: mocked.getDocument,
}));
vi.mock("../../lib/documents/fetch", () => ({ fetchPreviewBytes: mocked.fetch }));
afterEach(() => {
  vi.useRealTimers();
  vi.resetAllMocks();
});

it("destroys the owning task and ignores getPage when it resolves after timeout", async () => {
  let resolvePage!: (page: { getViewport: () => object; render: () => object }) => void;
  const latePage = {
    getViewport: vi.fn(() => ({ width: 320, height: 180 })),
    render: vi.fn(() => ({ promise: Promise.resolve(), cancel: () => {} })),
  };
  mocked.fetch.mockResolvedValue(new Uint8Array());
  mocked.getPage.mockImplementation(
    () =>
      new Promise((resolve) => {
        resolvePage = resolve;
      }),
  );
  mocked.getDocument.mockReturnValue({
    destroy: mocked.destroy,
    promise: Promise.resolve({
      numPages: 1,
      getPage: mocked.getPage,
      loadingTask: { destroy: mocked.destroy },
    }),
  });
  vi.useFakeTimers();
  render(
    <LocaleProvider>
      <PdfPreview src="/pdf" />
    </LocaleProvider>,
  );
  await vi.waitFor(() => expect(mocked.getPage).toHaveBeenCalledOnce());
  await act(() => vi.advanceTimersByTimeAsync(15_000));
  expect(screen.getByRole("alert")).toBeTruthy();
  expect(mocked.destroy).toHaveBeenCalledOnce();
  await act(async () => {
    resolvePage(latePage);
    await Promise.resolve();
  });
  expect(latePage.render).not.toHaveBeenCalled();
  expect(screen.getByRole("alert")).toBeTruthy();
});
it("does not start a PDF task when bytes arrive after the loading deadline", async () => {
  let resolveBytes!: (bytes: Uint8Array) => void;
  mocked.fetch.mockImplementation(
    () =>
      new Promise((resolve) => {
        resolveBytes = resolve;
      }),
  );
  vi.useFakeTimers();
  render(
    <LocaleProvider>
      <PdfPreview src="/slow-pdf" />
    </LocaleProvider>,
  );
  await vi.waitFor(() => expect(mocked.fetch).toHaveBeenCalledOnce());
  await act(() => vi.advanceTimersByTimeAsync(15_000));
  expect(screen.getByRole("alert")).toBeTruthy();
  await act(async () => {
    resolveBytes(new Uint8Array());
    await Promise.resolve();
  });
  expect(mocked.getDocument).not.toHaveBeenCalled();
});
it("shows the limit notice without building page slots for an over-limit PDF", async () => {
  const getPage = vi.fn();
  mocked.fetch.mockResolvedValue(new Uint8Array());
  mocked.getDocument.mockReturnValue({
    destroy: mocked.destroy,
    promise: Promise.resolve({
      numPages: 1001,
      getPage,
      loadingTask: { destroy: mocked.destroy },
    }),
  });
  const view = render(
    <LocaleProvider>
      <PdfPreview src="/many-pages" />
    </LocaleProvider>,
  );
  expect(await screen.findByRole("alert")).toBeTruthy();
  expect(screen.queryByRole("textbox", { name: "ページ番号" })).toBeNull();
  expect(view.container.querySelector("[data-pdf-page]")).toBeNull();
  expect(getPage).not.toHaveBeenCalled();
});

it("keeps rendered canvases intact while typing in the page input or stepping to buffered pages", async () => {
  const renders = new Map<number, number>();
  mocked.fetch.mockResolvedValue(new Uint8Array());
  mocked.getPage.mockImplementation(async (pageNumber: number) => ({
    cleanup: () => {},
    getViewport: ({ scale }: { scale: number }) => ({ width: 400 * scale, height: 600 * scale }),
    render: () => {
      renders.set(pageNumber, (renders.get(pageNumber) ?? 0) + 1);
      return { promise: Promise.resolve(), cancel: () => {} };
    },
  }));
  mocked.getDocument.mockReturnValue({
    destroy: mocked.destroy,
    promise: Promise.resolve({
      numPages: 3,
      getPage: mocked.getPage,
      loadingTask: { destroy: mocked.destroy },
    }),
  });
  const view = render(
    <LocaleProvider>
      <PdfPreview src="/three-pages" />
    </LocaleProvider>,
  );
  await vi.waitFor(() =>
    expect([renders.get(1), renders.get(2), renders.get(3)]).toEqual([1, 1, 1]),
  );
  const input = screen.getByRole("textbox", { name: "ページ番号" }) as HTMLInputElement;
  fireEvent.focus(input);
  fireEvent.change(input, { target: { value: "2" } });
  fireEvent.submit(input.closest("form")!);
  fireEvent.click(screen.getByRole("button", { name: "次のページ" }));
  await Promise.resolve();
  expect(mocked.fetch).toHaveBeenCalledOnce();
  expect([renders.get(1), renders.get(2), renders.get(3)]).toEqual([1, 1, 1]);
  const firstCanvas = view.container.querySelector(
    '[data-pdf-page="1"] canvas',
  ) as HTMLCanvasElement;
  expect(firstCanvas.width).toBeGreaterThan(0);
});
