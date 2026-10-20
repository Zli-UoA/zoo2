import { useTranslation } from "react-i18next";

import type { CSSProperties } from "react";
import { Toaster as Sonner, type ToasterProps } from "sonner";

import { useTheme } from "@/components/theme/ThemeProvider";

/**
 * sonner のホスト。`next-themes` は使わず、`ThemeProvider` の実効テーマを渡す。
 * 色は shadcn トークンへ合わせる。
 */
function Toaster(props: ToasterProps) {
  const { t } = useTranslation();
  const { resolvedTheme } = useTheme();

  return (
    <Sonner
      theme={resolvedTheme}
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
