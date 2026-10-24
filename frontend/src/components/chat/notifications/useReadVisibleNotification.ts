import { useEffect, useRef, type RefObject } from "react";

/** Only acknowledge cards actually seen in the foreground notifications pane. */
export function useReadVisibleNotification(
  ids: readonly string[],
  onRead: (ids: readonly string[]) => void,
  scrollRoot: RefObject<HTMLDivElement | null>,
) {
  const ref = useRef<HTMLLIElement>(null);
  useEffect(() => {
    const element = ref.current;
    if (!element || ids.length === 0 || typeof IntersectionObserver === "undefined") return;
    let visible = false;
    const read = () => {
      if (visible && document.visibilityState === "visible" && document.hasFocus()) onRead(ids);
    };
    const observer = new IntersectionObserver(
      (entries) => {
        visible = entries.some((entry) => entry.isIntersecting);
        read();
      },
      { root: scrollRoot.current, threshold: 0.1 },
    );
    observer.observe(element);
    document.addEventListener("visibilitychange", read);
    window.addEventListener("focus", read);
    return () => {
      observer.disconnect();
      document.removeEventListener("visibilitychange", read);
      window.removeEventListener("focus", read);
    };
  }, [ids, onRead, scrollRoot]);
  return ref;
}
