import { useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { mediaPreviewKind } from "../../lib/attachments";
import { imageAspectRatio, type ImageMetadata } from "../../lib/imageMetadata";
import { PreviewDownloadButton } from "../desktop/PreviewDownloadButton";
import { Button } from "./button";
import { Dialog, DialogContent, DialogTitle } from "./dialog";
import { ImagePreview } from "./image-preview";
import { DocumentPreview } from "./document-preview";
import { PdfPreview } from "./pdf-preview";
import { Spinner } from "./spinner";

/** Shared display and accessible enlargement for channel, encrypted and APub media. */
export function MediaPreview({
  src,
  name,
  contentType,
  loading,
  failed,
  onRetry,
  onError,
  onOpen,
  footer,
  initialOpen = false,
  embedded = false,
  depth = 0,
  ...metadata
}: ImageMetadata & {
  src?: string | null;
  name: string;
  contentType: string;
  loading?: boolean;
  failed?: boolean;
  onRetry?: () => void;
  onError?: () => void;
  onOpen?: () => void;
  footer?: ReactNode;
  initialOpen?: boolean;
  /** The native host already provides the preview window and close control. */
  embedded?: boolean;
  depth?: number;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(initialOpen);
  const [brokenSrc, setBrokenSrc] = useState<string | null>(null);
  const kind = mediaPreviewKind(contentType, name);
  const error = failed || (!!src && brokenSrc === src);
  const errorHandler = () => {
    setBrokenSrc(src ?? null);
    onError?.();
  };
  const downloadFooter =
    footer ??
    (src ? <PreviewDownloadButton id={`preview:${name}:${src}`} name={name} src={src} /> : null);
  function renderMedia(expanded: boolean) {
    if (error)
      return (
        <div role="alert" className="flex min-h-24 flex-col items-center justify-center gap-2 p-3">
          <p>{t("attachments.loadError")}</p>
          {onRetry ? (
            <Button
              variant="outline"
              size="sm"
              onClick={() => {
                setBrokenSrc(null);
                onRetry();
              }}
            >
              {t("attachments.retry")}
            </Button>
          ) : null}
        </div>
      );
    if (loading || !src)
      return kind === "image" ? (
        <div role="status" aria-label={t("attachments.loading")} aria-busy="true">
          <ImagePreview
            {...metadata}
            alt={name}
            className="w-full max-h-96"
            style={{ aspectRatio: imageAspectRatio(metadata) }}
          />
        </div>
      ) : (
        <Spinner label={t("attachments.loading")} className="min-h-32" />
      );
    if (kind === "image")
      return (
        <ImagePreview
          {...metadata}
          src={src}
          alt={name}
          className={
            expanded ? (embedded ? "h-full w-full" : "h-[75dvh] w-full") : "w-full max-h-96"
          }
          style={expanded ? undefined : { aspectRatio: imageAspectRatio(metadata) }}
          onError={errorHandler}
        />
      );
    if (kind === "video")
      return (
        <video
          key={src}
          src={src}
          controls
          playsInline
          preload="none"
          aria-label={name}
          className={
            expanded
              ? embedded
                ? "max-h-full w-full"
                : "h-[75dvh] w-full"
              : "aspect-video max-h-96 w-full"
          }
          onError={errorHandler}
        />
      );
    if (kind === "audio")
      return (
        <audio
          key={src}
          src={src}
          controls
          preload="none"
          aria-label={name}
          className="w-full min-w-0"
          onError={errorHandler}
        />
      );
    if (expanded && kind === "pdf") return <PdfPreview key={src} src={src} embedded={embedded} />;
    if (expanded && kind === "document")
      return (
        <DocumentPreview
          key={src}
          src={src}
          name={name}
          contentType={contentType}
          renderFile={
            depth < 3
              ? (file, url) => (
                  <MediaPreview
                    src={url}
                    name={file.name}
                    contentType={file.type}
                    depth={depth + 1}
                  />
                )
              : undefined
          }
        />
      );
    return null;
  }
  if (embedded)
    return (
      <div
        className={
          kind === "pdf"
            ? "flex h-dvh min-w-0 flex-col overflow-hidden bg-background p-2 text-foreground"
            : "h-dvh min-w-0 overflow-auto bg-background text-foreground"
        }
      >
        {renderMedia(true)}
      </div>
    );
  return (
    <div className="min-w-0 w-full overflow-hidden rounded-lg border bg-muted/30">
      <div
        style={
          kind === "image"
            ? { aspectRatio: imageAspectRatio(metadata), minHeight: 80, maxHeight: 384 }
            : undefined
        }
        className="w-full overflow-auto"
      >
        {kind === "image" && !error && src && !loading ? (
          <button
            type="button"
            className="block w-full cursor-zoom-in outline-none focus-visible:ring-2 focus-visible:ring-ring"
            aria-label={t("attachments.preview", { name })}
            onClick={() => {
              onOpen?.();
              setOpen(true);
            }}
          >
            {renderMedia(false)}
          </button>
        ) : // Unmount inline players while enlarged so there is only one active player.
        !open || (kind !== "video" && kind !== "audio") ? (
          renderMedia(false)
        ) : null}
      </div>
      <div className="flex min-w-0 flex-wrap items-center gap-2 p-2">
        <span className="min-w-0 flex-1 truncate text-xs text-muted-foreground" title={name}>
          {name}
        </span>
        {kind && kind !== "image" ? (
          <Button
            variant="outline"
            size="sm"
            className="max-w-full truncate"
            disabled={!src || loading}
            aria-label={t("attachments.preview", { name })}
            onClick={() => {
              onOpen?.();
              setOpen(true);
            }}
          >
            {t("attachments.preview", { name })}
          </Button>
        ) : null}
        {downloadFooter}
      </div>
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent className="w-[calc(100vw-1rem)] max-w-[calc(100vw-1rem)] gap-2 p-2 pt-12 sm:max-w-[min(96vw,80rem)]">
          <DialogTitle className="sr-only">{name}</DialogTitle>
          {open ? renderMedia(true) : null}
          <div className="flex min-w-0 items-center justify-between gap-2">
            <span className="min-w-0 truncate text-xs">{name}</span>
            {downloadFooter}
          </div>
        </DialogContent>
      </Dialog>
    </div>
  );
}
