import { useTranslation } from "react-i18next";
import { DownloadIcon, LoaderCircleIcon, PaperclipIcon } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { toast } from "sonner";

import { downloadAttachment } from "../../../lib/api/attachments";
import {
  formatAttachmentSize,
  isInlineImage,
  shouldRefreshAttachmentUrl,
} from "../../../lib/attachments";
import type { AttachmentRow } from "../../../lib/electric/types";
import { useAttachmentUrl, type AttachmentUrlState } from "../../../hooks/useAttachments";
import { useLocale } from "../../i18n/LocaleProvider";
import { Button } from "../../ui/button";
import { Dialog, DialogContent, DialogTitle } from "../../ui/dialog";
import { Spinner } from "../../ui/spinner";

export function MessageAttachments({ attachments }: { attachments: AttachmentRow[] }) {
  if (attachments.length === 0) {
    return null;
  }
  const images = attachments.filter((attachment) => isInlineImage(attachment.content_type));
  const files = attachments.filter((attachment) => !isInlineImage(attachment.content_type));
  return (
    <div className="mt-2 flex max-w-xl flex-col gap-2">
      {images.length > 0 ? (
        <div
          className={
            images.length === 1 ? "w-fit max-w-full" : "grid w-full max-w-sm grid-cols-2 gap-2"
          }
        >
          {images.map((attachment) => (
            <AttachmentImage
              key={attachment.id}
              attachment={attachment}
              multiple={images.length > 1}
            />
          ))}
        </div>
      ) : null}
      {files.length > 0 ? (
        <ul className="flex flex-col gap-1.5">
          {files.map((attachment) => (
            <li key={attachment.id}>
              <AttachmentFile attachment={attachment} />
            </li>
          ))}
        </ul>
      ) : null}
    </div>
  );
}

function AttachmentImage({
  attachment,
  multiple,
}: {
  attachment: AttachmentRow;
  multiple: boolean;
}) {
  const { t } = useTranslation();
  const state = useAttachmentUrl(attachment.id);
  const { status, url, reload } = state;
  const [broken, setBroken] = useState(false);
  const [open, setOpen] = useState(false);
  const [dialogFailed, setDialogFailed] = useState(false);
  const retried = useRef(false);
  const dialogRetried = useRef(false);

  useEffect(() => {
    if (url) {
      setBroken(false);
      setDialogFailed(false);
    }
  }, [url]);

  const thumbnail =
    broken || status === "error" ? (
      <AttachmentFileCard attachment={attachment} state={state} />
    ) : status === "loading" || !url ? (
      <div
        className={
          multiple
            ? "aspect-square w-full animate-pulse rounded-lg border bg-muted"
            : "h-48 w-64 max-w-full animate-pulse rounded-lg border bg-muted"
        }
        aria-label={t("attachments.loading")}
        aria-busy="true"
      />
    ) : (
      <button
        type="button"
        aria-label={t("attachments.preview", { name: attachment.file_name })}
        className={
          multiple
            ? "block aspect-square w-full cursor-zoom-in overflow-hidden rounded-lg border bg-muted outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
            : "block w-fit max-w-full cursor-zoom-in overflow-hidden rounded-lg border bg-muted outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
        }
        onClick={() => {
          dialogRetried.current = false;
          setDialogFailed(false);
          if (shouldRefreshAttachmentUrl(state.expiresAt)) {
            void state.getFreshUrl();
          }
          setOpen(true);
        }}
      >
        <img
          src={url}
          alt={attachment.file_name}
          loading="lazy"
          decoding="async"
          className={
            multiple
              ? "size-full object-cover transition-opacity hover:opacity-90"
              : "max-h-80 max-w-full object-contain transition-opacity hover:opacity-90"
          }
          onLoad={() => {
            retried.current = false;
          }}
          onError={() => {
            if (!retried.current) {
              retried.current = true;
              reload();
              return;
            }
            setBroken(true);
          }}
        />
      </button>
    );

  return (
    <>
      {thumbnail}
      <Dialog
        open={open}
        onOpenChange={(next) => {
          setOpen(next);
          if (!next) {
            setDialogFailed(false);
          }
        }}
      >
        <DialogContent className="w-auto max-w-[calc(100vw-2rem)] p-2 sm:max-w-[min(96vw,80rem)]">
          <DialogTitle className="sr-only">{attachment.file_name}</DialogTitle>
          {status === "error" || dialogFailed ? (
            <div className="flex min-h-48 flex-col items-center justify-center gap-3 px-4">
              <p role="alert" className="text-sm text-destructive">
                {t("attachments.loadError")}
              </p>
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={() => {
                  dialogRetried.current = false;
                  setDialogFailed(false);
                  reload();
                }}
              >
                {t("attachments.retry")}
              </Button>
            </div>
          ) : status === "loading" || !url ? (
            <Spinner label={t("attachments.loading")} className="min-h-64 min-w-64" />
          ) : (
            <img
              src={url}
              alt={attachment.file_name}
              className="mx-auto max-h-[78vh] max-w-full object-contain"
              onLoad={() => {
                dialogRetried.current = false;
                setDialogFailed(false);
              }}
              onError={() => {
                if (!dialogRetried.current) {
                  dialogRetried.current = true;
                  reload();
                  return;
                }
                setDialogFailed(true);
              }}
            />
          )}
          <div className="flex items-center justify-between gap-3 px-1">
            <span className="min-w-0 truncate text-xs text-muted-foreground">
              {attachment.file_name}
            </span>
            <AttachmentDownloadButton attachment={attachment} state={state} showLabel />
          </div>
        </DialogContent>
      </Dialog>
    </>
  );
}

function AttachmentFile({ attachment }: { attachment: AttachmentRow }) {
  const state = useAttachmentUrl(attachment.id);
  return <AttachmentFileCard attachment={attachment} state={state} />;
}

function AttachmentFileCard({
  attachment,
  state,
}: {
  attachment: AttachmentRow;
  state: AttachmentUrlState;
}) {
  const { t } = useTranslation();
  const { locale } = useLocale();
  const { status, reload } = state;
  const label =
    status === "error"
      ? t("attachments.loadError")
      : status === "loading"
        ? t("attachments.loading")
        : null;

  return (
    <div className="flex items-center gap-2 rounded-lg border bg-muted/30 px-2.5 py-2">
      <PaperclipIcon className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
      <div className="min-w-0 flex-1">
        <p className="truncate text-sm font-medium">{attachment.file_name}</p>
        <p className="text-xs text-muted-foreground">
          {formatAttachmentSize(attachment.file_size, locale)}
          {label ? ` · ${label}` : null}
        </p>
      </div>
      {status === "loading" ? (
        <LoaderCircleIcon
          className="size-4 shrink-0 animate-spin text-muted-foreground"
          aria-hidden="true"
        />
      ) : null}
      {status === "error" ? (
        <button
          type="button"
          className="text-xs font-medium text-link hover:underline"
          onClick={reload}
        >
          {t("attachments.retry")}
        </button>
      ) : null}
      <AttachmentDownloadButton attachment={attachment} state={state} />
    </div>
  );
}

function AttachmentDownloadButton({
  attachment,
  state,
  showLabel = false,
}: {
  attachment: AttachmentRow;
  state: AttachmentUrlState;
  showLabel?: boolean;
}) {
  const { t } = useTranslation();
  const [downloading, setDownloading] = useState(false);

  async function download() {
    if (downloading) {
      return;
    }
    setDownloading(true);
    try {
      const url = await state.getFreshUrl();
      if (!url) {
        throw new Error("Attachment URL is unavailable");
      }
      await downloadAttachment(url, attachment.file_name);
    } catch {
      toast.error(t("attachments.downloadError"));
    } finally {
      setDownloading(false);
    }
  }

  const label = t("attachments.download", { name: attachment.file_name });
  return (
    <Button
      type="button"
      variant="ghost"
      size={showLabel ? "sm" : "icon-sm"}
      className={showLabel ? "shrink-0 gap-1 text-xs" : "text-muted-foreground"}
      aria-label={label}
      title={label}
      disabled={downloading}
      onClick={() => void download()}
    >
      {downloading ? (
        <LoaderCircleIcon className="size-4 animate-spin" aria-hidden="true" />
      ) : (
        <DownloadIcon className="size-4" aria-hidden="true" />
      )}
      {showLabel ? t("attachments.downloadShort") : null}
    </Button>
  );
}
