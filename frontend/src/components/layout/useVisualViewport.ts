import { useLayoutEffect, useRef } from "react";

/** ソフトキーボード表示時も、シェルを実際に見えている領域へ収める。 */
export function useVisualViewport() {
  const ref = useRef<HTMLDivElement>(null);

  useLayoutEffect(() => {
    const viewport = window.visualViewport;
    const element = ref.current;
    if (!viewport || !element) return;

    const update = () => {
      // ピンチズーム中はレイアウトを縮めず、ブラウザの拡大操作に任せる。
      if (viewport.scale !== 1) return;
      element.style.height = `${viewport.height}px`;
      element.style.top = `${viewport.offsetTop}px`;
    };
    update();
    viewport.addEventListener("resize", update);
    viewport.addEventListener("scroll", update);
    return () => {
      viewport.removeEventListener("resize", update);
      viewport.removeEventListener("scroll", update);
      element.style.removeProperty("height");
      element.style.removeProperty("top");
    };
  }, []);

  return ref;
}
