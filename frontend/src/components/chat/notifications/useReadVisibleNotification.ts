import { useEffect, useRef, useState, type RefObject } from "react";

/** Only acknowledge cards actually seen in the foreground notifications pane. */
export function useReadVisibleNotification(
  ids: readonly string[],
  onRead: (ids: readonly string[]) => void,
  scrollRoot: RefObject<HTMLDivElement | null>,
  enabled = true,
) {
  const ref = useRef<HTMLLIElement>(null);
  const [hasBeenVisible, setHasBeenVisible] = useState(false);
  const [isVisible, setIsVisible] = useState<boolean | null>(null);
  useEffect(() => {
    const element = ref.current;
    if (!element) return;
    if (typeof IntersectionObserver === "undefined") {
      setHasBeenVisible(true);
      setIsVisible(true);
      return;
    }
    let visible = false;
    const read = () => {
      if (
        enabled &&
        visible &&
        ids.length > 0 &&
        document.visibilityState === "visible" &&
        document.hasFocus()
      )
        onRead(ids);
    };
    const observer = new IntersectionObserver(
      (entries) => {
        visible = entries.some((entry) => entry.isIntersecting);
        setIsVisible(visible);
        if (visible) setHasBeenVisible(true);
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
  }, [ids, onRead, scrollRoot, enabled]);
  return { ref, hasBeenVisible, isVisible };
}
