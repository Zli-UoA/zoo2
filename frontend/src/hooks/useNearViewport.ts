import { useEffect, useRef, useState, type RefObject } from "react";

/**
 * 要素が、スクロールする親の表示範囲の上下 200px 以内にあり、かつタブが表示されているか。
 * 全行をマウントしたまま並べる画面（Home・公開プロフィールなど）で、画面付近の行だけが取得を始めるために使う。
 * `IntersectionObserver` のない環境では、常に近いものとして扱う。
 *
 * 返した `ref` を、判定する要素に付ける。
 */
export function useNearViewport<T extends Element>(): [RefObject<T | null>, boolean] {
  const ref = useRef<T>(null);
  const [nearby, setNearby] = useState(false);
  const [visible, setVisible] = useState(
    () => typeof document !== "undefined" && document.visibilityState !== "hidden",
  );
  useEffect(() => {
    const update = () => setVisible(document.visibilityState !== "hidden");
    update();
    document.addEventListener("visibilitychange", update);
    if (typeof IntersectionObserver === "undefined") {
      setNearby(true);
      return () => document.removeEventListener("visibilitychange", update);
    }
    // rootMargin must expand the actual scrolling pane, not only the window.
    let root = ref.current?.parentElement ?? null;
    while (root && root !== document.body && root !== document.documentElement) {
      if (/(auto|scroll|overlay)/.test(getComputedStyle(root).overflowY)) break;
      root = root.parentElement;
    }
    if (root === document.body || root === document.documentElement) root = null;
    const observer = new IntersectionObserver(
      ([entry]) => setNearby(entry?.isIntersecting ?? false),
      { root, rootMargin: "200px 0px" },
    );
    if (ref.current) observer.observe(ref.current);
    return () => {
      observer.disconnect();
      document.removeEventListener("visibilitychange", update);
    };
  }, []);
  return [ref, nearby && visible];
}
