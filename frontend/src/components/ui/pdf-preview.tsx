import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { PDFDocumentProxy } from "pdfjs-dist";
import { useTranslation } from "react-i18next";
import { fetchPreviewBytes } from "../../lib/documents/fetch";
import { Button } from "./button";
import { Input } from "./input";
import { Spinner } from "./spinner";

type PageSize = { width: number; height: number };

const RENDER_BUFFER_PAGES = 2;
const PAGE_TIMEOUT_MS = 15_000;

export function parsePdfPageNumber(raw: string, total: number): number | null {
  if (total < 1) return null;
  const normalized = raw
    .trim()
    .replace(/[０-９]/g, (char) => String.fromCharCode(char.charCodeAt(0) - 0xfee0));
  if (!/^\d+$/.test(normalized)) return null;
  const parsed = Number.parseInt(normalized, 10);
  if (!Number.isFinite(parsed)) return null;
  return Math.min(Math.max(1, parsed), total);
}

function isRenderCancelled(error: unknown): boolean {
  return (
    typeof error === "object" &&
    error !== null &&
    "name" in error &&
    (error as { name?: unknown }).name === "RenderingCancelledException"
  );
}

/** The same canvas viewer in browsers and Android, without depending on PDF plugins. */
export function PdfPreview({ src, embedded = false }: { src: string; embedded?: boolean }) {
  const { t } = useTranslation();
  const scrollRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const prevBtnRef = useRef<HTMLButtonElement>(null);
  const nextBtnRef = useRef<HTMLButtonElement>(null);

  const pageRefs = useRef(new Map<number, HTMLDivElement>());
  const canvasRefs = useRef(new Map<number, HTMLCanvasElement>());
  const renderedCanvasesRef = useRef(new Map<number, HTMLCanvasElement>());
  const kickRenderRef = useRef<(() => void) | null>(null);

  const inputFocusedRef = useRef(false);
  const justFocusedRef = useRef(false);
  const selectOnRenderRef = useRef(false);
  const dirtyInputRef = useRef(false);
  const composingRef = useRef(false);
  const lastEnterComposingRef = useRef(false);
  const programmaticScrollTopRef = useRef<number | null>(null);
  const lastAlignedScrollTopRef = useRef<number | null>(null);
  const pendingAnchorRef = useRef<number | null>(null);

  const [document, setDocument] = useState<PDFDocumentProxy | null>(null);
  const [failed, setFailed] = useState(false);
  const [primaryReady, setPrimaryReady] = useState(false);
  const [page, setPage] = useState(1);
  const [pageInput, setPageInput] = useState("1");
  const [defaultSize, setDefaultSize] = useState<PageSize | null>(null);
  const [pageSizes, setPageSizes] = useState<Record<number, PageSize>>({});
  const [visibleRange, setVisibleRange] = useState<{ start: number; end: number }>({
    start: 1,
    end: 1,
  });

  const pageRef = useRef(page);
  pageRef.current = page;

  const totalPages = document?.numPages ?? 1;

  const keptPages = useMemo(() => {
    const start = Math.max(1, visibleRange.start - RENDER_BUFFER_PAGES);
    const end = Math.min(totalPages, visibleRange.end + RENDER_BUFFER_PAGES);
    const ordered: number[] = [];
    const seen = new Set<number>();
    const add = (value: number) => {
      if (value < 1 || value > totalPages || seen.has(value)) return;
      seen.add(value);
      ordered.push(value);
    };

    // Always prioritize the current page and visible range, then buffer pages nearby.
    add(page);
    for (let p = visibleRange.start; p <= Math.min(totalPages, visibleRange.end); p++) add(p);
    for (let p = start; p <= end; p++) add(p);
    return ordered;
  }, [page, totalPages, visibleRange.end, visibleRange.start]);

  const keptPagesSet = useMemo(() => new Set(keptPages), [keptPages]);
  const keptPagesRef = useRef(keptPages);
  keptPagesRef.current = keptPages;
  const keptPagesSetRef = useRef(keptPagesSet);
  keptPagesSetRef.current = keptPagesSet;
  const visibleRangeRef = useRef(visibleRange);
  visibleRangeRef.current = visibleRange;

  const updatePageDisplay = useCallback((nextPage: number) => {
    setPage(nextPage);
    if (!dirtyInputRef.current) {
      setPageInput(String(nextPage));
    }
  }, []);

  const measureViewport = useCallback(
    (syncCurrentPage: boolean) => {
      const container = scrollRef.current;
      if (!container || !document) return;
      const clientHeight = container.clientHeight;
      if (clientHeight <= 0) return;

      const containerRect = container.getBoundingClientRect();
      let firstVisible: number | null = null;
      let lastVisible: number | null = null;
      let bestPage: number | null = null;
      let bestOverlap = -1;

      for (let p = 1; p <= document.numPages; p++) {
        const element = pageRefs.current.get(p);
        if (!element) continue;
        const rect = element.getBoundingClientRect();
        if (rect.height <= 0) continue;
        const relTop = rect.top - containerRect.top;
        const relBottom = rect.bottom - containerRect.top;
        if (relBottom > 0 && relTop < clientHeight) {
          if (firstVisible === null) firstVisible = p;
          lastVisible = p;
          const overlap =
            Math.min(rect.bottom, containerRect.bottom) - Math.max(rect.top, containerRect.top);
          if (overlap > bestOverlap) {
            bestOverlap = overlap;
            bestPage = p;
          }
        }
      }

      if (firstVisible !== null && lastVisible !== null) {
        setVisibleRange((prev) =>
          prev.start === firstVisible && prev.end === lastVisible
            ? prev
            : { start: firstVisible, end: lastVisible },
        );
      }

      if (!syncCurrentPage) return;

      const { scrollTop, scrollHeight } = container;
      let detected = bestPage;
      if (scrollTop <= 2) {
        detected = 1;
      } else if (scrollHeight > clientHeight + 2 && scrollTop + clientHeight >= scrollHeight - 2) {
        detected = document.numPages;
      }
      if (detected !== null) {
        updatePageDisplay(detected);
      }
    },
    [document, updatePageDisplay],
  );

  const alignScrollToPage = useCallback((targetPage: number) => {
    const container = scrollRef.current;
    const pageElement = pageRefs.current.get(targetPage);
    if (!container || !pageElement) return;
    const containerRect = container.getBoundingClientRect();
    const pageRect = pageElement.getBoundingClientRect();
    if (containerRect.height <= 0 || pageRect.height <= 0) return;
    const paddingTop = Number.parseFloat(getComputedStyle(container).paddingTop) || 0;
    const nextTop = Math.max(
      0,
      container.scrollTop + (pageRect.top - containerRect.top) - paddingTop,
    );
    const previousTop = container.scrollTop;
    container.scrollTop = nextTop;
    lastAlignedScrollTopRef.current = container.scrollTop;
    if (Math.abs(container.scrollTop - previousTop) > 1) {
      programmaticScrollTopRef.current = container.scrollTop;
    }
  }, []);

  const jumpToPage = useCallback(
    (target: number) => {
      if (!document) return;
      const clamped = Math.min(Math.max(1, target), document.numPages);
      dirtyInputRef.current = false;
      setPage(clamped);
      setPageInput(String(clamped));
      pendingAnchorRef.current = pageSizes[clamped] ? null : clamped;
      alignScrollToPage(clamped);
      setVisibleRange((prev) =>
        clamped >= prev.start && clamped <= prev.end ? prev : { start: clamped, end: clamped },
      );
      measureViewport(false);
      if (inputFocusedRef.current) {
        if (pageInput !== String(clamped)) {
          selectOnRenderRef.current = true;
        }
        inputRef.current?.select();
      }
    },
    [alignScrollToPage, document, measureViewport, pageInput, pageSizes],
  );

  useEffect(() => {
    if (selectOnRenderRef.current && inputFocusedRef.current) {
      selectOnRenderRef.current = false;
      inputRef.current?.select();
    }
  }, [pageInput]);

  const commitPageInput = useCallback(() => {
    dirtyInputRef.current = false;
    if (!document) {
      setPageInput(String(pageRef.current));
      return;
    }
    const parsed = parsePdfPageNumber(pageInput, document.numPages);
    if (parsed === null) {
      setPageInput(String(pageRef.current));
      return;
    }
    jumpToPage(parsed);
  }, [document, jumpToPage, pageInput]);

  const clearProgrammaticAnchor = useCallback(() => {
    pendingAnchorRef.current = null;
    programmaticScrollTopRef.current = null;
    lastAlignedScrollTopRef.current = null;
  }, []);

  const handleScroll = useCallback(() => {
    const container = scrollRef.current;
    if (!container || !document) return;
    if (programmaticScrollTopRef.current !== null) {
      const diff = Math.abs(container.scrollTop - programmaticScrollTopRef.current);
      programmaticScrollTopRef.current = null;
      if (diff <= 2) {
        measureViewport(false);
        return;
      }
    }
    pendingAnchorRef.current = null;
    lastAlignedScrollTopRef.current = null;
    measureViewport(true);
  }, [document, measureViewport]);

  // Load PDF document once per src.
  useEffect(() => {
    let cancelled = false;
    let cleanup = () => {};
    const abort = new AbortController();
    setDocument(null);
    setFailed(false);
    setPrimaryReady(false);
    setPage(1);
    setPageInput("1");
    dirtyInputRef.current = false;
    justFocusedRef.current = false;
    setDefaultSize(null);
    setPageSizes({});
    setVisibleRange({ start: 1, end: 1 });
    renderedCanvasesRef.current.clear();
    pendingAnchorRef.current = null;
    programmaticScrollTopRef.current = null;
    lastAlignedScrollTopRef.current = null;

    const timeout = setTimeout(() => {
      cancelled = true;
      abort.abort();
      cleanup();
      setFailed(true);
    }, PAGE_TIMEOUT_MS);

    void (async () => {
      const [pdfjs, bytes] = await Promise.all([
        import("pdfjs-dist/legacy/build/pdf.mjs"),
        fetchPreviewBytes(src, abort.signal),
      ]);
      if (cancelled) return;
      // デコーダー・フォントと同じ版付きのディレクトリから、module worker として読み込む。
      // 以前のハッシュ付き .mjs のURLとは別になるため、octet-stream でキャッシュされた
      // 古い応答も使われない（#631）。
      const assets = new URL(
        `${import.meta.env.BASE_URL}assets/pdfjs/${pdfjs.version}/`,
        window.location.href,
      );
      pdfjs.GlobalWorkerOptions.workerSrc = new URL("pdf.worker.min.mjs", assets).href;
      const task = pdfjs.getDocument({
        data: bytes,
        cMapUrl: new URL("cmaps/", assets).href,
        cMapPacked: true,
        wasmUrl: new URL("wasm/", assets).href,
        standardFontDataUrl: new URL("standard_fonts/", assets).href,
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

  // Sequential prioritized page renderer with 15s timeout per active render task.
  useEffect(() => {
    if (!document) return;
    let cancelled = false;
    let wake: (() => void) | null = null;
    let activePage: number | null = null;
    let activeCancel: (() => void) | null = null;

    const pickNextPage = (): number | null => {
      for (const candidate of keptPagesRef.current) {
        const canvas = canvasRefs.current.get(candidate);
        if (!canvas) continue;
        if (renderedCanvasesRef.current.get(candidate) === canvas) continue;
        return candidate;
      }
      return null;
    };

    const kick = () => {
      if (cancelled) return;
      if (activePage !== null) {
        const stillKept = keptPagesSetRef.current.has(activePage);
        const vis = visibleRangeRef.current;
        const activeIsVisible = activePage >= vis.start && activePage <= vis.end;
        const hasWaitingVisible = keptPagesRef.current.some((candidate) => {
          if (candidate < vis.start || candidate > vis.end) return false;
          const canvas = canvasRefs.current.get(candidate);
          return canvas && renderedCanvasesRef.current.get(candidate) !== canvas;
        });
        if (!stillKept || (!activeIsVisible && hasWaitingVisible)) {
          activeCancel?.();
        }
      }
      const resolve = wake;
      wake = null;
      resolve?.();
    };
    kickRenderRef.current = kick;

    void (async () => {
      while (!cancelled) {
        const targetPage = pickNextPage();
        if (targetPage === null) {
          await new Promise<void>((resolve) => {
            wake = resolve;
          });
          continue;
        }

        let taskAborted = false;
        let renderCancel: (() => void) | null = null;
        activePage = targetPage;
        activeCancel = () => {
          taskAborted = true;
          renderCancel?.();
        };

        const timeout = setTimeout(() => {
          if (cancelled || taskAborted) return;
          cancelled = true;
          taskAborted = true;
          renderCancel?.();
          void document.loadingTask.destroy();
          setFailed(true);
          setPrimaryReady(true);
          const resolve = wake;
          wake = null;
          resolve?.();
        }, PAGE_TIMEOUT_MS);

        try {
          const pdfPage = await document.getPage(targetPage);
          if (cancelled || taskAborted) {
            clearTimeout(timeout);
            continue;
          }
          const natural = pdfPage.getViewport({ scale: 1 });
          const viewport = pdfPage.getViewport({
            scale: Math.min(2, 1600 / natural.width, 2400 / natural.height),
          });
          const element = canvasRefs.current.get(targetPage);
          if (cancelled || taskAborted || !element) {
            clearTimeout(timeout);
            continue;
          }
          const size: PageSize = {
            width: Math.max(1, Math.ceil(viewport.width)),
            height: Math.max(1, Math.ceil(viewport.height)),
          };
          element.width = size.width;
          element.height = size.height;

          const render = pdfPage.render({ canvas: element, viewport });
          renderCancel = () => render.cancel();
          if (taskAborted) {
            render.cancel();
            clearTimeout(timeout);
            continue;
          }
          await render.promise;
          clearTimeout(timeout);
          if (cancelled || taskAborted) continue;

          renderedCanvasesRef.current.set(targetPage, element);
          setPageSizes((prev) =>
            prev[targetPage]?.width === size.width && prev[targetPage]?.height === size.height
              ? prev
              : { ...prev, [targetPage]: size },
          );
          if (targetPage === 1) {
            setDefaultSize((prev) =>
              prev?.width === size.width && prev?.height === size.height ? prev : size,
            );
          }
          setPrimaryReady(true);
        } catch (error) {
          clearTimeout(timeout);
          if (cancelled) break;
          if (taskAborted && isRenderCancelled(error)) {
            continue;
          }
          if (isRenderCancelled(error)) {
            continue;
          }
          setFailed(true);
          setPrimaryReady(true);
          break;
        } finally {
          activePage = null;
          activeCancel = null;
        }
      }
    })();

    return () => {
      cancelled = true;
      kickRenderRef.current = null;
      activeCancel?.();
      const resolve = wake;
      wake = null;
      resolve?.();
    };
  }, [document]);

  // Wake renderer whenever the active window or canvas refs change.
  useEffect(() => {
    kickRenderRef.current?.();
  }, [document, keptPages]);

  // Re-align scroll anchor or re-measure visible pages when layout sizes settle or container resizes.
  useEffect(() => {
    if (!document) return;
    const container = scrollRef.current;
    const anchor = pendingAnchorRef.current;
    if (anchor !== null && container) {
      const expectedTop = lastAlignedScrollTopRef.current;
      if (expectedTop !== null && Math.abs(container.scrollTop - expectedTop) > 2) {
        pendingAnchorRef.current = null;
        lastAlignedScrollTopRef.current = null;
      } else {
        alignScrollToPage(anchor);
        if (pageSizes[anchor]) {
          pendingAnchorRef.current = null;
        }
      }
    }
    measureViewport(false);
  }, [alignScrollToPage, defaultSize, document, measureViewport, pageSizes]);

  useEffect(() => {
    const container = scrollRef.current;
    if (!container || !document || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(() => {
      measureViewport(false);
    });
    observer.observe(container);
    return () => observer.disconnect();
  }, [document, measureViewport]);

  const setPageElement = useCallback((pageNumber: number, element: HTMLDivElement | null) => {
    if (element) {
      pageRefs.current.set(pageNumber, element);
    } else {
      pageRefs.current.delete(pageNumber);
    }
  }, []);

  const setCanvasElement = useCallback((pageNumber: number, element: HTMLCanvasElement | null) => {
    const existing = canvasRefs.current.get(pageNumber);
    if (existing && existing !== element) {
      existing.width = 0;
      existing.height = 0;
      renderedCanvasesRef.current.delete(pageNumber);
    }
    if (element) {
      canvasRefs.current.set(pageNumber, element);
      kickRenderRef.current?.();
    } else {
      canvasRefs.current.delete(pageNumber);
      renderedCanvasesRef.current.delete(pageNumber);
    }
  }, []);

  const loading = !failed && (!document || !primaryReady);

  return (
    <div
      className={embedded ? "flex min-h-0 flex-1 flex-col gap-2" : "flex min-w-0 flex-col gap-2"}
    >
      <div className="flex shrink-0 flex-wrap items-center justify-center gap-2 sm:gap-3">
        <Button
          ref={prevBtnRef}
          type="button"
          variant="outline"
          size="sm"
          disabled={!document || page <= 1 || failed}
          onClick={() => jumpToPage(page - 1)}
        >
          {t("attachments.previousPage")}
        </Button>
        <form
          className="flex items-center gap-1.5"
          onSubmit={(event) => {
            event.preventDefault();
            if (!composingRef.current && !lastEnterComposingRef.current) {
              commitPageInput();
            }
            lastEnterComposingRef.current = false;
          }}
        >
          <Input
            ref={inputRef}
            type="text"
            inputMode="numeric"
            pattern="[0-9]*"
            enterKeyHint="go"
            disabled={!document || failed}
            value={pageInput}
            aria-label={t("attachments.pageNumber")}
            className="h-7 w-14 px-1.5 text-center tabular-nums"
            onCompositionStart={() => {
              composingRef.current = true;
            }}
            onCompositionEnd={() => {
              composingRef.current = false;
            }}
            onKeyDown={(event) => {
              justFocusedRef.current = false;
              if (event.key === "Enter") {
                lastEnterComposingRef.current =
                  event.nativeEvent.isComposing || composingRef.current;
              }
            }}
            onFocus={(event) => {
              inputFocusedRef.current = true;
              justFocusedRef.current = true;
              dirtyInputRef.current = false;
              event.currentTarget.select();
            }}
            onMouseUp={(event) => {
              if (justFocusedRef.current) {
                event.preventDefault();
              }
            }}
            onClick={(event) => {
              if (justFocusedRef.current || !dirtyInputRef.current) {
                justFocusedRef.current = false;
                event.currentTarget.select();
              }
            }}
            onChange={(event) => {
              selectOnRenderRef.current = false;
              dirtyInputRef.current = true;
              setPageInput(event.target.value);
            }}
            onBlur={(event) => {
              inputFocusedRef.current = false;
              justFocusedRef.current = false;
              const nextTarget = event.relatedTarget as Node | null;
              if (
                nextTarget &&
                (prevBtnRef.current?.contains(nextTarget) ||
                  nextBtnRef.current?.contains(nextTarget))
              ) {
                dirtyInputRef.current = false;
                setPageInput(String(pageRef.current));
                return;
              }
              if (dirtyInputRef.current) {
                commitPageInput();
              }
            }}
          />
          <span className="text-sm text-muted-foreground tabular-nums">
            / {document?.numPages ?? "—"}
          </span>
        </form>
        <Button
          ref={nextBtnRef}
          type="button"
          variant="outline"
          size="sm"
          disabled={!document || page >= document.numPages || failed}
          onClick={() => jumpToPage(page + 1)}
        >
          {t("attachments.nextPage")}
        </Button>
        <span className="sr-only" aria-live="polite">
          {document ? t("attachments.pdfPage", { page }) : ""}
        </span>
      </div>
      {failed ? (
        <p role="alert" className="text-center text-sm text-destructive">
          {t("attachments.loadError")}
        </p>
      ) : (
        <div
          ref={scrollRef}
          role="region"
          aria-label={t("attachments.pdfPages")}
          tabIndex={0}
          onScroll={handleScroll}
          onWheel={clearProgrammaticAnchor}
          onPointerDown={clearProgrammaticAnchor}
          onTouchStart={clearProgrammaticAnchor}
          onKeyDown={clearProgrammaticAnchor}
          className={
            embedded
              ? "relative min-h-0 flex-1 overflow-auto rounded bg-muted/50 p-3 outline-none focus-visible:ring-2 focus-visible:ring-ring"
              : "relative max-h-[min(65dvh,calc(100dvh-10rem))] min-h-32 overflow-auto rounded bg-muted/50 p-3 outline-none focus-visible:ring-2 focus-visible:ring-ring"
          }
        >
          {loading ? (
            <div className="pointer-events-none absolute inset-0 z-10 flex items-center justify-center bg-background/60">
              <Spinner label={t("attachments.loading")} />
            </div>
          ) : null}
          <div className="flex flex-col items-center gap-3">
            {Array.from({ length: totalPages }, (_, index) => {
              const pageNumber = index + 1;
              const size = pageSizes[pageNumber] ?? defaultSize;
              const slotStyle = size
                ? { aspectRatio: `${size.width} / ${size.height}`, maxWidth: `${size.width}px` }
                : { maxWidth: "100%", minHeight: "12rem" };
              const mounted = keptPagesSet.has(pageNumber);
              return (
                <div
                  key={pageNumber}
                  ref={(element) => setPageElement(pageNumber, element)}
                  data-pdf-page={pageNumber}
                  style={slotStyle}
                  className="relative mx-auto w-full overflow-hidden rounded-xs bg-white shadow-xs"
                >
                  {mounted ? (
                    <canvas
                      ref={(element) => setCanvasElement(pageNumber, element)}
                      role="img"
                      aria-label={t("attachments.pdfPage", { page: pageNumber })}
                      className="block size-full"
                    />
                  ) : (
                    <div
                      role="img"
                      aria-label={t("attachments.pdfPage", { page: pageNumber })}
                      className="size-full bg-white"
                    />
                  )}
                </div>
              );
            })}
          </div>
        </div>
      )}
    </div>
  );
}
