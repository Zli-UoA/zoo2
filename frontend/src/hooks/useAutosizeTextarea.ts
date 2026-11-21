import { useEffect, useLayoutEffect, useRef, type RefObject } from "react";

/** Fits the field to its draft. Returns false, and leaves the field alone, while it is not rendered. */
function fit(element: HTMLTextAreaElement, maxHeight: number): boolean {
  // A field inside a hidden parent (`display: none`) has no `scrollHeight`: fitting it would set 0.
  if (element.getClientRects().length === 0) return false;
  element.style.height = "auto";
  element.style.height = `${Math.min(element.scrollHeight, maxHeight)}px`;
  element.style.overflowY = element.scrollHeight > maxHeight ? "auto" : "hidden";
  return true;
}

/** Fits the field and records the width it was fitted at (0 when it was not rendered: still to do). */
function refit(element: HTMLTextAreaElement, maxHeight: number, fitted: { current: number }) {
  fitted.current = fit(element, maxHeight) ? element.offsetWidth : 0;
}

/**
 * Grow with the draft, then scroll within the configured input height. The field is fitted again
 * when the draft changes, when it was hidden and is shown, and when its width changes (the same
 * draft wraps onto more or fewer lines). `offsetWidth` is what is compared, not `clientWidth`: a
 * scrollbar that fitting adds or removes must not count, and fitting changes the height only.
 */
export function useAutosizeTextarea(
  ref: RefObject<HTMLTextAreaElement | null>,
  value: string,
  maxHeight: number,
) {
  // The width the field was last fitted at; 0 while a fit is pending (it was not rendered).
  const fittedWidth = useRef(0);

  useLayoutEffect(() => {
    const element = ref.current;
    if (element) refit(element, maxHeight, fittedWidth);
  }, [ref, value, maxHeight]);

  // After every render, so that what only re-renders the component still fits: the parent that shows
  // a hidden field again, and a parent that changes the field's width.
  useLayoutEffect(() => {
    const element = ref.current;
    if (element && element.offsetWidth !== fittedWidth.current)
      refit(element, maxHeight, fittedWidth);
  });

  // A change of width without a render (a sidebar that opens, a window that is resized). Nothing is
  // changed inside the callback: changing the observed element there makes the browser report "loop
  // completed with undelivered notifications" and drop the notification. The next frame fits.
  useEffect(() => {
    const element = ref.current;
    if (!element || typeof ResizeObserver === "undefined") return;
    let frame = 0;
    const observer = new ResizeObserver(() => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        if (element.offsetWidth !== fittedWidth.current) refit(element, maxHeight, fittedWidth);
      });
    });
    observer.observe(element);
    return () => {
      cancelAnimationFrame(frame);
      observer.disconnect();
    };
  }, [ref, maxHeight]);
}
