import { SensitiveMedia } from "../../ui/sensitive-media";
import { MediaPreview } from "../../ui/media-preview";
import { useTranslation } from "react-i18next";
import { LoaderCircleIcon, PaperclipIcon } from "lucide-react";
import { useRef } from "react";

import { useDesktopSave } from "../../../hooks/useDesktopSave";
import { AttachmentDownloadError, downloadAttachmentWithRetry } from "../../../lib/api/attachments";
import {
  type AttachmentMetadata,
  formatAttachmentSize,
  mediaPreviewKind,
  shouldRefreshAttachmentUrl,
} from "../../../lib/attachments";
import { useAttachmentUrl, type AttachmentUrlState } from "../../../hooks/useAttachments";
import { useLocale } from "../../i18n/LocaleProvider";
import { DownloadProgressButton } from "../../desktop/DownloadProgressButton";

export function MessageAttachments({ attachments }: { attachments: AttachmentMetadata[] }) {
  if (attachments.length === 0) {
    return null;
  }
  const previews = attachments.filter((attachment) =>
    mediaPreviewKind(attachment.content_type, attachment.file_name),
  );
  const files = attachments.filter(
    (attachment) => !mediaPreviewKind(attachment.content_type, attachment.file_name),
  );
  return (
    <div className="mt-2 flex max-w-xl flex-col gap-2">
      {previews.length > 0 ? (
        <div className="grid w-full grid-cols-1 gap-2">
          {previews.map((attachment) => (
            <SensitiveMedia
              key={`${attachment.id}:${Boolean(attachment.is_sensitive)}`}
              sensitive={attachment.is_sensitive}
              preview={attachment}
              className="w-full"
            >
              <AttachmentPreview attachment={attachment} />
            </SensitiveMedia>
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

function AttachmentPreview({ attachment }: { attachment: AttachmentMetadata }) {
  const state = useAttachmentUrl(attachment.id);
  const retried = useRef(false);
  return (
    <MediaPreview
      {...attachment}
      src={state.url}
      name={attachment.file_name}
      contentType={attachment.content_type}
      loading={state.status === "loading"}
      failed={state.status === "error"}
      onRetry={() => {
        retried.current = false;
        state.reload();
      }}
      onError={() => {
        if (!retried.current) {
          retried.current = true;
          state.reload();
        }
      }}
      onOpen={() => {
        if (shouldRefreshAttachmentUrl(state.expiresAt)) {
          // 表示用の先読み。失敗してもダウンロード時に再試行する。
          void state.getFreshUrl().catch(() => undefined);
        }
      }}
      footer={<AttachmentDownloadButton attachment={attachment} state={state} showLabel />}
    />
  );
}

function AttachmentFile({ attachment }: { attachment: AttachmentMetadata }) {
  const state = useAttachmentUrl(attachment.id);
  return <AttachmentFileCard attachment={attachment} state={state} />;
}

function AttachmentFileCard({
  attachment,
  state,
}: {
  attachment: AttachmentMetadata;
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
        <p className="truncate text-xs text-muted-foreground">
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
  attachment: AttachmentMetadata;
  state: AttachmentUrlState;
  showLabel?: boolean;
}) {
  const { t } = useTranslation();
  const {
    state: desktopState,
    pending,
    download,
    cancel,
  } = useDesktopSave({
    id: attachment.id,
    fileName: attachment.file_name,
    source: {
      kind: "url",
      resolveUrl: state.getFreshUrl,
      size: Number(attachment.file_size),
    },
    fallback: async () => {
      await downloadAttachmentWithRetry(state.getFreshUrl, attachment.file_name);
    },
    onFailure: (error) => {
      // 失敗の理由（URL・通信・HTTP）と状態コードを、問い合わせ時に追えるように残す。
      console.error("添付のダウンロードに失敗しました", {
        attachmentId: attachment.id,
        reason: error instanceof AttachmentDownloadError ? error.reason : "unknown",
        status: error instanceof AttachmentDownloadError ? error.status : null,
        error,
      });
    },
  });

  return (
    <DownloadProgressButton
      state={desktopState}
      pending={pending}
      showLabel={showLabel}
      downloadLabel={t("attachments.download", { name: attachment.file_name })}
      downloadShortLabel={t("attachments.downloadShort")}
      cancelLabel={t("attachments.cancelDownload", { name: attachment.file_name })}
      cancelShortLabel={t("attachments.cancelDownloadShort")}
      finishingLabel={t("attachments.saving")}
      progressLabel={t("attachments.downloading")}
      onDownload={() => void download()}
      onCancel={cancel}
    />
  );
}
