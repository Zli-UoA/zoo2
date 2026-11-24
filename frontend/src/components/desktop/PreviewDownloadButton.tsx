import { useCallback, useEffect, useId, useState } from "react";
import { useTranslation } from "react-i18next";

import { useDesktopSave } from "../../hooks/useDesktopSave";
import { AttachmentDownloadError, downloadAttachmentWithRetry } from "../../lib/api/attachments";
import { ExternalLink } from "../ui/external-link";
import { DownloadProgressButton } from "./DownloadProgressButton";

/**
 * `footer` を持たないプレビューの保存ボタン。デスクトップ版では共通の保存経路を使い、
 * Web と古いアプリでは今どおりのリンクを出す。
 *
 * 外部の HTTPS を CORS なしで取得できないときは、案内を出して別タブのリンクに切り替える。
 */
export function PreviewDownloadButton({
  name,
  src,
  size,
  showLabel = false,
  className = "text-xs text-link",
}: {
  name: string;
  src: string;
  size?: number | null;
  showLabel?: boolean;
  className?: string;
}) {
  const { t } = useTranslation();
  const id = useId();
  const [useExternalLink, setUseExternalLink] = useState(false);
  useEffect(() => setUseExternalLink(false), [src]);
  const resolveUrl = useCallback(async () => src, [src]);
  const { state, pending, available, download, cancel } = useDesktopSave({
    id,
    fileName: name,
    source: { kind: "url", resolveUrl, size },
    fallback: async () => {
      await downloadAttachmentWithRetry(resolveUrl, name);
    },
    // CORS やオフラインで本文が読めないときだけ、別タブのリンクへ案内する。
    // URL の取得や HTTP のエラーは通常のダウンロード失敗として伝える。
    describeError: (error) =>
      error instanceof AttachmentDownloadError && error.reason === "network"
        ? { message: t("attachments.previewSaveUnsupported") }
        : undefined,
    onFailure: (error) => {
      if (error instanceof AttachmentDownloadError && error.reason === "network") {
        setUseExternalLink(true);
      }
    },
  });

  if (useExternalLink || available === false) {
    return src.startsWith("blob:") ? (
      <a href={src} download={name} className={className}>
        {t("attachments.downloadShort")}
      </a>
    ) : (
      <ExternalLink href={src} className={className}>
        {t("attachments.downloadShort")}
      </ExternalLink>
    );
  }

  return (
    <DownloadProgressButton
      state={state}
      pending={pending}
      showLabel={showLabel}
      downloadLabel={t("attachments.download", { name })}
      downloadShortLabel={t("attachments.downloadShort")}
      cancelLabel={t("attachments.cancelDownload", { name })}
      cancelShortLabel={t("attachments.cancelDownloadShort")}
      finishingLabel={t("attachments.saving")}
      progressLabel={t("attachments.downloading")}
      onDownload={() => void download()}
      onCancel={cancel}
    />
  );
}
