import { useEffect, useLayoutEffect, type RefObject } from "react";

function fit(element: HTMLTextAreaElement, maxHeight: number) {
  element.style.height = "auto";
  element.style.height = `${Math.min(element.scrollHeight, maxHeight)}px`;
  element.style.overflowY = element.scrollHeight > maxHeight ? "auto" : "hidden";
}

/** Grow with the draft, then scroll within the configured input height. */
export function useAutosizeTextarea(
  ref: RefObject<HTMLTextAreaElement | null>,
  value: string,
  maxHeight: number,
) {
  useLayoutEffect(() => {
    const element = ref.current;
    if (element) fit(element, maxHeight);
  }, [ref, value, maxHeight]);

  // A hidden textarea (`display: none`) has no `scrollHeight`, so a draft set meanwhile would be
  // fitted to 0 and stay that small after it is shown. A change of width also rewraps the draft.
  // Only the width is watched: fitting changes the height, which must not fit again.
  useEffect(() => {
    const element = ref.current;
    if (!element || typeof ResizeObserver === "undefined") return;
    let width = element.clientWidth;
    const observer = new ResizeObserver(() => {
      if (element.clientWidth === width) return;
      width = element.clientWidth;
      if (width > 0) fit(element, maxHeight);
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, [ref, maxHeight]);
}
