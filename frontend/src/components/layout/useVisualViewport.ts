import { useLayoutEffect, useState } from "react";

/** ソフトキーボード表示時も、シェルや全画面ダイアログを実際に見えている領域へ収める。 */
export function useVisualViewport(active = true) {
  const [element, setElement] = useState<HTMLDivElement | null>(null);

  useLayoutEffect(() => {
    const viewport = window.visualViewport;
    if (!active || !viewport || !element) return;

    const update = () => {
      // ピンチズーム中はレイアウトを縮めず、ブラウザの拡大操作に任せる。
      if (viewport.scale !== 1) return;
      // キーボードが出ていなければ h-dvh / top-0 に任せる。iOS のバウンスで
      // 動いた offsetTop を拾うと、シェルごと指に追従してずれるため。
      if (window.innerHeight - viewport.height < 1) {
        element.style.removeProperty("height");
        element.style.removeProperty("top");
        if (window.scrollY !== 0) window.scrollTo(0, 0);
        return;
      }
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
  }, [active, element]);

  return setElement;
}
