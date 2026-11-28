import {
  memo,
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
} from "react";
import type { PDFDocumentProxy, PDFPageProxy } from "pdfjs-dist";
import { useTranslation } from "react-i18next";
import { fetchPreviewBytes } from "../../lib/documents/fetch";
import { Button } from "./button";
import { Input } from "./input";
import { Spinner } from "./spinner";

type PageSize = { width: number; height: number };

const RENDER_BUFFER_PAGES = 2;
const PAGE_TIMEOUT_MS = 15_000;
// 全ページ分の枠を並べるため、ページ数に上限を設ける。
const MAX_PDF_PREVIEW_PAGES = 1_000;

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

const PdfPageSlot = memo(function PdfPageSlot({
  pageNumber,
  label,
  size,
  mounted,
  onSlotMount,
  onSlotUnmount,
  onCanvasMount,
  onCanvasUnmount,
}: {
  pageNumber: number;
  label: string;
  size: PageSize | null;
  mounted: boolean;
  onSlotMount: (pageNumber: number, element: HTMLDivElement) => void;
  onSlotUnmount: (pageNumber: number, element: HTMLDivElement) => void;
  onCanvasMount: (pageNumber: number, element: HTMLCanvasElement) => void;
  onCanvasUnmount: (pageNumber: number, element: HTMLCanvasElement) => void;
}) {
  const slotRef = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const element = slotRef.current;
    if (!element) return;
    onSlotMount(pageNumber, element);
    return () => onSlotUnmount(pageNumber, element);
  }, [onSlotMount, onSlotUnmount, pageNumber]);

  const slotStyle = size
    ? ({
        "--pdf-aspect": `${size.width} / ${size.height}`,
        "--pdf-width": `${size.width}px`,
      } as CSSProperties)
    : undefined;

  return (
    <div
      ref={slotRef}
      data-pdf-page={pageNumber}
      aria-hidden={mounted ? undefined : true}
      style={slotStyle}
      className={
        size
          ? "relative mx-auto aspect-(--pdf-aspect) w-full max-w-(--pdf-width) overflow-hidden rounded-xs bg-white shadow-xs"
          : "relative mx-auto w-full max-w-full min-h-48 overflow-hidden rounded-xs bg-white shadow-xs"
      }
    >
      {mounted ? (
        <PdfPageCanvas
          pageNumber={pageNumber}
          label={label}
          onMount={onCanvasMount}
          onUnmount={onCanvasUnmount}
        />
      ) : null}
    </div>
  );
});

function PdfPageCanvas({
  pageNumber,
  label,
  onMount,
  onUnmount,
}: {
  pageNumber: number;
  label: string;
  onMount: (pageNumber: number, element: HTMLCanvasElement) => void;
  onUnmount: (pageNumber: number, element: HTMLCanvasElement) => void;
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  useLayoutEffect(() => {
    const element = canvasRef.current;
    if (!element) return;
    onMount(pageNumber, element);
    return () => onUnmount(pageNumber, element);
  }, [onMount, onUnmount, pageNumber]);

  return <canvas ref={canvasRef} role="img" aria-label={label} className="block size-full" />;
}

/** The same canvas viewer in browsers and Android, without depending on PDF plugins. */
export function PdfPreview({ src, embedded = false }: { src: string; embedded?: boolean }) {
  const { t } = useTranslation();
  const totalPagesId = useId();
  const scrollRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const pageRefs = useRef(new Map<number, HTMLDivElement>());
  const canvasRefs = useRef(new Map<number, HTMLCanvasElement>());
  const renderedCanvasesRef = useRef(new Map<number, HTMLCanvasElement>());
  const pageProxiesRef = useRef(new Map<number, PDFPageProxy>());
  const kickRenderRef = useRef<(() => void) | null>(null);
  const pageButtonPressRef = useRef(false);

  const inputFocusedRef = useRef(false);
  const justFocusedRef = useRef(false);
  const selectOnRenderRef = useRef(false);
  const dirtyInputRef = useRef(false);
  const composingRef = useRef(false);
  const compositionEndedRef = useRef(false);
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
  const overLimit = totalPages > MAX_PDF_PREVIEW_PAGES;

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
      const viewportTop = containerRect.top;
      const viewportBottom = viewportTop + clientHeight;

      let low = 1;
      let high = document.numPages;
      let startPage = document.numPages + 1;
      while (low <= high) {
        const mid = (low + high) >> 1;
        const element = pageRefs.current.get(mid);
        if (!element) {
          startPage = 1;
          break;
        }
        const rect = element.getBoundingClientRect();
        if (rect.bottom > viewportTop) {
          startPage = mid;
          high = mid - 1;
        } else {
          low = mid + 1;
        }
      }

      let firstVisible: number | null = null;
      let lastVisible: number | null = null;
      let bestPage: number | null = null;
      let bestOverlap = -1;

      for (let p = startPage; p <= document.numPages; p++) {
        const element = pageRefs.current.get(p);
        if (!element) continue;
        const rect = element.getBoundingClientRect();
        if (rect.height <= 0) continue;
        if (rect.top >= viewportBottom) break;
        if (rect.bottom > viewportTop) {
          if (firstVisible === null) firstVisible = p;
          lastVisible = p;
          const overlap = Math.min(rect.bottom, viewportBottom) - Math.max(rect.top, viewportTop);
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
      pendingAnchorRef.current = clamped;
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
    [alignScrollToPage, document, measureViewport, pageInput],
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
    pageProxiesRef.current.clear();
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

  const releasePageProxy = useCallback((pageNumber: number) => {
    const proxy = pageProxiesRef.current.get(pageNumber);
    if (!proxy) return;
    proxy.cleanup();
    pageProxiesRef.current.delete(pageNumber);
  }, []);

  // Sequential prioritized page renderer with 15s timeout per active render task.
  useEffect(() => {
    if (!document || overLimit) return;
    let cancelled = false;
    let wake: (() => void) | null = null;
    let activePage: number | null = null;
    let activeCancel: (() => void) | null = null;
    let renderCancel: (() => void) | null = null;

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
        activePage = targetPage;
        activeCancel = () => {
          taskAborted = true;
          renderCancel?.();
        };

        const timeout = setTimeout(() => {
          // 取得中に保持範囲外へ出ても期限は残し、取得完了時に描画だけを省略する。
          // 描画をキャンセルした場合は既に render.cancel() が済んでいるので期限は不要。
          if (cancelled || (taskAborted && renderCancel)) return;
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
          if (cancelled) {
            clearTimeout(timeout);
            break;
          }
          pageProxiesRef.current.set(targetPage, pdfPage);
          const natural = pdfPage.getViewport({ scale: 1 });
          const viewport = pdfPage.getViewport({
            scale: Math.min(2, 1600 / natural.width, 2400 / natural.height),
          });
          const size: PageSize = {
            width: Math.max(1, Math.ceil(viewport.width)),
            height: Math.max(1, Math.ceil(viewport.height)),
          };
          setPageSizes((prev) =>
            prev[targetPage]?.width === size.width && prev[targetPage]?.height === size.height
              ? prev
              : { ...prev, [targetPage]: size },
          );
          setDefaultSize((prev) => prev ?? size);

          const element = canvasRefs.current.get(targetPage);
          if (taskAborted || !element) {
            clearTimeout(timeout);
            releasePageProxy(targetPage);
            continue;
          }
          element.width = size.width;
          element.height = size.height;

          const render = pdfPage.render({ canvas: element, viewport });
          renderCancel = () => render.cancel();
          if (taskAborted) {
            render.cancel();
            clearTimeout(timeout);
            releasePageProxy(targetPage);
            continue;
          }
          await render.promise;
          clearTimeout(timeout);
          if (cancelled || taskAborted) continue;

          renderedCanvasesRef.current.set(targetPage, element);
          setPrimaryReady(true);
        } catch (error) {
          clearTimeout(timeout);
          if (cancelled) break;
          if (taskAborted || isRenderCancelled(error)) {
            releasePageProxy(targetPage);
            continue;
          }
          setFailed(true);
          setPrimaryReady(true);
          break;
        } finally {
          activePage = null;
          activeCancel = null;
          renderCancel = null;
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
  }, [document, overLimit, releasePageProxy]);

  // Wake renderer whenever the active window changes.
  useEffect(() => {
    kickRenderRef.current?.();
  }, [document, keptPages]);

  // Re-align scroll anchor before paint when page slot dimensions settle.
  useLayoutEffect(() => {
    if (!document || overLimit) return;
    const container = scrollRef.current;
    const anchor = pendingAnchorRef.current;
    if (anchor !== null && container) {
      const expectedTop = lastAlignedScrollTopRef.current;
      const anchorElement = pageRefs.current.get(anchor);
      const anchorRect = anchorElement?.getBoundingClientRect();
      const containerRect = container.getBoundingClientRect();
      const paddingTop = Number.parseFloat(getComputedStyle(container).paddingTop) || 0;
      // ページ高の確定で上方が縮むと scrollTop がクランプされ、対象ページが画面上部から
      // 押し出される。この場合はユーザー操作ではないので、アンカーを捨てず再整列する。
      const clampedByLayout =
        expectedTop !== null &&
        container.scrollTop < expectedTop &&
        anchorRect !== undefined &&
        anchorRect.top - containerRect.top - paddingTop < -2;
      if (
        expectedTop === null ||
        clampedByLayout ||
        Math.abs(container.scrollTop - expectedTop) <= 2
      ) {
        alignScrollToPage(anchor);
      } else {
        pendingAnchorRef.current = null;
        lastAlignedScrollTopRef.current = null;
      }
    }
    measureViewport(false);
  }, [alignScrollToPage, defaultSize, document, measureViewport, overLimit, pageSizes]);

  useEffect(() => {
    const container = scrollRef.current;
    if (!container || !document || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(() => {
      const anchor = pendingAnchorRef.current;
      if (anchor !== null) {
        // 幅の変化で枠の高さが変わっても、ジャンプ先を基準に整列してから測定する。
        alignScrollToPage(anchor);
        measureViewport(false);
        return;
      }
      // ジャンプ中でなければリサイズ後の表示へ追従させる。
      measureViewport(true);
    });
    observer.observe(container);
    return () => observer.disconnect();
  }, [alignScrollToPage, document, measureViewport]);

  const handleSlotMount = useCallback((pageNumber: number, element: HTMLDivElement) => {
    pageRefs.current.set(pageNumber, element);
  }, []);

  const handleSlotUnmount = useCallback((pageNumber: number, element: HTMLDivElement) => {
    if (pageRefs.current.get(pageNumber) === element) {
      pageRefs.current.delete(pageNumber);
    }
  }, []);

  const handleCanvasMount = useCallback((pageNumber: number, element: HTMLCanvasElement) => {
    canvasRefs.current.set(pageNumber, element);
    kickRenderRef.current?.();
  }, []);

  const handleCanvasUnmount = useCallback(
    (pageNumber: number, element: HTMLCanvasElement) => {
      if (canvasRefs.current.get(pageNumber) === element) {
        canvasRefs.current.delete(pageNumber);
      }
      if (renderedCanvasesRef.current.get(pageNumber) === element) {
        renderedCanvasesRef.current.delete(pageNumber);
      }
      element.width = 0;
      element.height = 0;
      if (!keptPagesSetRef.current.has(pageNumber)) {
        releasePageProxy(pageNumber);
      }
    },
    [releasePageProxy],
  );

  // ページ枠はページ数分必要だが、入力欄の変更では作り直さない。
  const pageSlots = useMemo(
    () =>
      overLimit
        ? []
        : Array.from({ length: totalPages }, (_, index) => {
            const pageNumber = index + 1;
            return (
              <PdfPageSlot
                key={pageNumber}
                pageNumber={pageNumber}
                label={t("attachments.pdfPage", { page: pageNumber })}
                size={pageSizes[pageNumber] ?? defaultSize}
                mounted={keptPagesSet.has(pageNumber)}
                onSlotMount={handleSlotMount}
                onSlotUnmount={handleSlotUnmount}
                onCanvasMount={handleCanvasMount}
                onCanvasUnmount={handleCanvasUnmount}
              />
            );
          }),
    [
      defaultSize,
      handleCanvasMount,
      handleCanvasUnmount,
      handleSlotMount,
      handleSlotUnmount,
      keptPagesSet,
      overLimit,
      pageSizes,
      t,
      totalPages,
    ],
  );

  if (overLimit) {
    return (
      <p role="alert" className="py-6 text-center text-sm">
        {t("attachments.previewLimit")}
      </p>
    );
  }

  const loading = !failed && (!document || !primaryReady);

  return (
    <div
      className={embedded ? "flex min-h-0 flex-1 flex-col gap-2" : "flex min-w-0 flex-col gap-2"}
    >
      <div className="flex shrink-0 flex-wrap items-center justify-center gap-2 sm:gap-3">
        <Button
          type="button"
          variant="outline"
          size="sm"
          disabled={!document || page <= 1 || failed}
          onPointerDown={() => {
            pageButtonPressRef.current = true;
          }}
          onPointerCancel={() => {
            pageButtonPressRef.current = false;
          }}
          onClick={() => {
            pageButtonPressRef.current = false;
            jumpToPage(page - 1);
          }}
        >
          {t("attachments.previousPage")}
        </Button>
        <form
          noValidate
          className="flex items-center gap-1.5"
          onSubmit={(event) => {
            event.preventDefault();
            if (!composingRef.current && !lastEnterComposingRef.current) {
              commitPageInput();
            }
            lastEnterComposingRef.current = false;
            compositionEndedRef.current = false;
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
            aria-describedby={document ? totalPagesId : undefined}
            className="h-7 w-16 px-1.5 text-center tabular-nums"
            onCompositionStart={() => {
              composingRef.current = true;
              compositionEndedRef.current = false;
            }}
            onCompositionEnd={() => {
              composingRef.current = false;
              compositionEndedRef.current = true;
            }}
            onKeyDown={(event) => {
              justFocusedRef.current = false;
              // Safari では compositionend が keydown より先に来る。確定直後の keyCode 229 だけを
              // IME 確定とみなし、Android のソフトキーボードの常時 229 は通常の Enter として扱う。
              // 時間ではなく次のキー入力で解除するため、端末による遅延の影響を受けない。
              const justEndedComposition = compositionEndedRef.current;
              compositionEndedRef.current = false;
              if (event.key === "Enter") {
                lastEnterComposingRef.current =
                  event.nativeEvent.isComposing ||
                  composingRef.current ||
                  (justEndedComposition && event.nativeEvent.keyCode === 229);
              }
            }}
            onFocus={(event) => {
              inputFocusedRef.current = true;
              justFocusedRef.current = true;
              dirtyInputRef.current = false;
              // ボタン外でポインタを離して click が来なかった場合の押下フラグを戻す。
              pageButtonPressRef.current = false;
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
            onBlur={() => {
              inputFocusedRef.current = false;
              justFocusedRef.current = false;
              const pressedPageButton = pageButtonPressRef.current;
              pageButtonPressRef.current = false;
              // Safari ではボタン_click_で relatedTarget が null になるため、
              // ポインタ操作の有無で下書きの破棄を判断する。Tab 移動では値を確定する。
              if (pressedPageButton) {
                dirtyInputRef.current = false;
                setPageInput(String(pageRef.current));
                return;
              }
              if (dirtyInputRef.current) {
                commitPageInput();
              }
            }}
          />
          <span id={totalPagesId} className="text-sm tabular-nums">
            / {document?.numPages ?? "—"}
          </span>
        </form>
        <Button
          type="button"
          variant="outline"
          size="sm"
          disabled={!document || page >= document.numPages || failed}
          onPointerDown={() => {
            pageButtonPressRef.current = true;
          }}
          onPointerCancel={() => {
            pageButtonPressRef.current = false;
          }}
          onClick={() => {
            pageButtonPressRef.current = false;
            jumpToPage(page + 1);
          }}
        >
          {t("attachments.nextPage")}
        </Button>
        <span className="sr-only" aria-live="polite">
          {document ? t("attachments.pdfPage", { page }) : ""}
        </span>
      </div>
      {failed ? (
        <p role="alert" className="py-6 text-center text-sm">
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
          <div className="flex flex-col items-center gap-3">{pageSlots}</div>
        </div>
      )}
    </div>
  );
}
