import { useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { imageAspectRatio, type ImageMetadata } from "../../lib/imageMetadata";
import { Button } from "./button";
import { ImagePreview } from "./image-preview";

export function SensitiveMedia({
  sensitive,
  preview = {},
  className = "",
  children,
}: {
  sensitive?: boolean;
  preview?: ImageMetadata;
  className?: string;
  children: ReactNode;
}) {
  const { t } = useTranslation();
  const [revealed, setRevealed] = useState(false);
  if (!sensitive) return children;
  return (
    <div className={`min-w-0 ${className}`}>
      {revealed ? (
        <>
          <Button
            type="button"
            variant="outline"
            size="sm"
            className="mb-2 h-auto max-w-full whitespace-normal px-2 py-1 text-xs"
            aria-expanded="true"
            onClick={() => setRevealed(false)}
          >
            {t("attachments.hideSensitive")}
          </Button>
          {children}
        </>
      ) : (
        <Button
          type="button"
          variant="ghost"
          className="relative block h-auto w-full overflow-hidden rounded-lg border p-0 whitespace-normal"
          style={{ aspectRatio: imageAspectRatio(preview), minHeight: 80, maxHeight: 384 }}
          aria-label={t("attachments.showSensitive")}
          title={t("attachments.sensitive")}
          aria-expanded="false"
          onClick={() => setRevealed(true)}
        >
          <ImagePreview
            {...preview}
            src={null}
            alt=""
            className="pointer-events-none absolute inset-0 size-full"
          />
          <span className="absolute inset-0 flex flex-col items-center justify-center gap-1 p-2">
            <span className="rounded bg-background/85 px-2 py-1 text-balance text-xs text-foreground">
              {t("attachments.showSensitive")}
            </span>
          </span>
        </Button>
      )}
    </div>
  );
}
