import { act, render, screen } from "@testing-library/react";
import { afterEach, beforeAll, expect, it, vi } from "vite-plus/test";
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
beforeAll(async () => {
  await import("pdfjs-dist/legacy/build/pdf.worker.min.mjs?url");
});
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
