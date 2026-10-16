import { useEffect, useState } from "react";

/**
 * CSS メディアクエリの一致を購読する。SSR と初回クライアント描画は `false` を
 * 返し、マウント後に実際の値を反映する（ハイドレーションのずれを避ける）。
 */
export function useMediaQuery(query: string): boolean {
  const [matches, setMatches] = useState(false);

  useEffect(() => {
    if (typeof window === "undefined" || typeof window.matchMedia !== "function") {
      return;
    }
    const mediaQuery = window.matchMedia(query);
    const update = () => setMatches(mediaQuery.matches);
    update();
    mediaQuery.addEventListener("change", update);
    return () => mediaQuery.removeEventListener("change", update);
  }, [query]);

  return matches;
}
