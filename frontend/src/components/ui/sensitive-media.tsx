import { useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "./button";

export function SensitiveMedia({
  sensitive,
  children,
}: {
  sensitive?: boolean;
  children: ReactNode;
}) {
  const { t } = useTranslation();
  const [revealed, setRevealed] = useState(false);
  if (!sensitive) return children;
  return (
    <div className="flex min-h-24 min-w-0 flex-col items-start gap-2 rounded-lg border bg-muted p-2">
      <Button
        type="button"
        variant="outline"
        size="sm"
        className="h-auto max-w-full whitespace-normal px-2 py-1 text-xs"
        aria-expanded={revealed}
        onClick={() => setRevealed((value) => !value)}
      >
        {t(revealed ? "attachments.hideSensitive" : "attachments.showSensitive")}
      </Button>
      {revealed ? (
        children
      ) : (
        <p className="text-balance text-xs text-muted-foreground">{t("attachments.sensitive")}</p>
      )}
    </div>
  );
}
