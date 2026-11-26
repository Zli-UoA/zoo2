import { useTranslation } from "react-i18next";

import type { CSSProperties } from "react";
import { Toaster as Sonner, type ToasterProps } from "sonner";

import { useMediaQuery } from "@/components/layout/useMediaQuery";
import { useTheme } from "@/components/theme/ThemeProvider";

/**
 * sonner のホスト。`next-themes` は使わず、`ThemeProvider` の実効テーマを渡す。
 * 色は shadcn トークンへ合わせる。
 *
 * タッチ端末では既定の下部表示が composer を覆うため、上部中央に出す。
 * デスクトップは sonner の既定位置を維持する。
 */
function Toaster(props: ToasterProps) {
  const { t } = useTranslation();
  const { resolvedTheme } = useTheme();
  const isTouch = useMediaQuery("(hover: none)");

  return (
    <Sonner
      theme={resolvedTheme}
      position={isTouch ? "top-center" : undefined}
      containerAriaLabel={t("notifications.label")}
      toastOptions={{ closeButtonAriaLabel: t("notifications.close") }}
      className="toaster group"
      style={
        {
          "--normal-bg": "var(--popover)",
          "--normal-text": "var(--popover-foreground)",
          "--normal-border": "var(--border)",
        } as CSSProperties
      }
      {...props}
    />
  );
}

export { Toaster };
