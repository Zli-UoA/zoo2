import { useLayoutEffect, useRef, type RefObject } from "react";

/** Fits the field to its draft. Returns false, and leaves the field alone, while it is not rendered. */
function fit(element: HTMLTextAreaElement, maxHeight: number): boolean {
  // A field inside a hidden parent (`display: none`) has no `scrollHeight`: fitting it would set 0.
  if (element.getClientRects().length === 0) return false;
  element.style.height = "auto";
  element.style.height = `${Math.min(element.scrollHeight, maxHeight)}px`;
  element.style.overflowY = element.scrollHeight > maxHeight ? "auto" : "hidden";
  return true;
}

/** Grow with the draft, then scroll within the configured input height. */
export function useAutosizeTextarea(
  ref: RefObject<HTMLTextAreaElement | null>,
  value: string,
  maxHeight: number,
) {
  const pending = useRef(false);
  useLayoutEffect(() => {
    const element = ref.current;
    if (element) pending.current = !fit(element, maxHeight);
  }, [ref, value, maxHeight]);

  // A draft set while the field was hidden is fitted by the render that shows it again (the parent
  // that shows it renders this component too). No dependencies: it must look after every render.
  useLayoutEffect(() => {
    const element = ref.current;
    if (element && pending.current) pending.current = !fit(element, maxHeight);
  });
}
