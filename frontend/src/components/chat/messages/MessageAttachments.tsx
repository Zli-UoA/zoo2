import { useTranslation } from "react-i18next";
import { DownloadIcon, LoaderCircleIcon, PaperclipIcon } from "lucide-react";
import { useEffect, useRef, useState } from "react";

import { formatAttachmentSize, isInlineImage } from "../../../lib/attachments";
import type { AttachmentRow } from "../../../lib/electric/types";
import { useAttachmentUrl, type AttachmentUrlState } from "../../../hooks/useAttachments";
import { useLocale } from "../../i18n/LocaleProvider";
import { ExternalLink } from "../../ui/external-link";

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
  const { status, url, reload } = useAttachmentUrl(attachment.id);
  const [broken, setBroken] = useState(false);
  const retried = useRef(false);

  useEffect(() => {
    if (url) {
      setBroken(false);
    }
  }, [url]);

  if (broken || status === "error") {
    return <AttachmentFileCard attachment={attachment} state={{ status, url, reload }} />;
  }

  if (status === "loading" || !url) {
    return (
      <div
        className={
          multiple
            ? "aspect-square w-full animate-pulse rounded-lg border bg-muted"
            : "h-48 w-64 max-w-full animate-pulse rounded-lg border bg-muted"
        }
        aria-label={t("attachments.loading")}
        aria-busy="true"
      />
    );
  }

  return (
    <ExternalLink
      href={url}
      aria-label={t("attachments.open", { name: attachment.file_name })}
      className={
        multiple
          ? "block aspect-square overflow-hidden rounded-lg border bg-muted"
          : "block w-fit max-w-full overflow-hidden rounded-lg border bg-muted"
      }
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
    </ExternalLink>
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
  const { status, url, reload } = state;
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
      {url ? (
        <ExternalLink
          href={url}
          download={attachment.file_name}
          aria-label={t("attachments.download", { name: attachment.file_name })}
          title={t("attachments.download", { name: attachment.file_name })}
          className="rounded p-1 text-muted-foreground hover:bg-muted hover:text-foreground"
        >
          <DownloadIcon className="size-4" />
        </ExternalLink>
      ) : null}
    </div>
  );
}
