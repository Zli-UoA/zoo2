import { useEffect, useRef, useState } from "react";
import type { PDFDocumentProxy } from "pdfjs-dist";
import { useTranslation } from "react-i18next";
import { fetchPreviewBytes } from "../../lib/documents/fetch";
import { Button } from "./button";
import { Spinner } from "./spinner";

/** The same canvas viewer in browsers and Android, without depending on PDF plugins. */
export function PdfPreview({ src }: { src: string }) {
  const { t } = useTranslation();
  const canvas = useRef<HTMLCanvasElement>(null);
  const [page, setPage] = useState(1);
  const [document, setDocument] = useState<PDFDocumentProxy | null>(null);
  const [failed, setFailed] = useState(false);
  const [rendering, setRendering] = useState(false);
  useEffect(() => {
    let cancelled = false;
    let cleanup = () => {};
    const abort = new AbortController();
    const timeout = setTimeout(() => {
      abort.abort();
      cleanup();
      setFailed(true);
    }, 15_000);
    void (async () => {
      const [pdfjs, workerUrl, bytes] = await Promise.all([
        import("pdfjs-dist/legacy/build/pdf.mjs"),
        import("pdfjs-dist/legacy/build/pdf.worker.min.mjs?url"),
        fetchPreviewBytes(src, abort.signal),
      ]);
      if (cancelled) return;
      pdfjs.GlobalWorkerOptions.workerSrc = workerUrl.default;
      const task = pdfjs.getDocument({
        data: bytes,
        maxImageSize: 16_000_000,
        useSystemFonts: true,
      });
      cleanup = () => {
        void task.destroy();
      };
      const pdf = await task.promise;
      if (!cancelled) {
        clearTimeout(timeout);
        setDocument(pdf);
      }
    })().catch(() => {
      if (!cancelled) {
        clearTimeout(timeout);
        setFailed(true);
      }
    });
    return () => {
      cancelled = true;
      clearTimeout(timeout);
      abort.abort();
      cleanup();
    };
  }, [src]);
  useEffect(() => {
    if (!document) return;
    let cancelled = false;
    let cleanup = () => {};
    setRendering(true);
    const timeout = setTimeout(() => {
      cleanup();
      setFailed(true);
      setRendering(false);
    }, 15_000);
    void (async () => {
      const pdfPage = await document.getPage(page);
      const natural = pdfPage.getViewport({ scale: 1 });
      const viewport = pdfPage.getViewport({
        scale: Math.min(2, 1600 / natural.width, 2400 / natural.height),
      });
      const element = canvas.current;
      if (cancelled || !element) return;
      element.width = Math.ceil(viewport.width);
      element.height = Math.ceil(viewport.height);
      const render = pdfPage.render({ canvas: element, viewport });
      cleanup = () => render.cancel();
      await render.promise;
      if (!cancelled) {
        clearTimeout(timeout);
        setRendering(false);
      }
    })().catch(() => {
      if (!cancelled) {
        clearTimeout(timeout);
        setFailed(true);
        setRendering(false);
      }
    });
    return () => {
      cancelled = true;
      clearTimeout(timeout);
      cleanup();
    };
  }, [document, page]);
  const loading = !failed && (!document || rendering);
  return (
    <div className="min-w-0 space-y-2">
      <div className="flex flex-wrap items-center justify-center gap-3">
        <Button
          variant="outline"
          size="sm"
          disabled={page <= 1 || loading || failed}
          onClick={() => setPage((p) => p - 1)}
        >
          {t("attachments.previousPage")}
        </Button>
        <span aria-live="polite">
          {page} / {document?.numPages ?? "—"}
        </span>
        <Button
          variant="outline"
          size="sm"
          disabled={!document || page >= document.numPages || loading || failed}
          onClick={() => setPage((p) => p + 1)}
        >
          {t("attachments.nextPage")}
        </Button>
      </div>
      {loading ? <Spinner label={t("attachments.loading")} /> : null}
      {failed ? <p role="alert">{t("attachments.loadError")}</p> : null}
      <div className="max-h-[65dvh] overflow-auto rounded bg-white">
        <canvas
          ref={canvas}
          role="img"
          aria-label={t("attachments.pdfPage", { page })}
          className="mx-auto h-auto max-w-full"
        />
      </div>
    </div>
  );
}
