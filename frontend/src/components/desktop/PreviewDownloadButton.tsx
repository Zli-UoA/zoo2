import { useCallback, useId, useState } from "react";
import { useTranslation } from "react-i18next";

import { useDesktopSave, type DesktopSavePrepared } from "../../hooks/useDesktopSave";
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
  id,
  name,
  src,
  size,
  showLabel = false,
  className = "text-xs text-link",
}: {
  /** 同じファイルを表示する複数の場所で進捗と中止を共有するための識別子。 */
  id?: string;
  name: string;
  src: string;
  size?: number | null;
  showLabel?: boolean;
  className?: string;
}) {
  const { t } = useTranslation();
  const generatedId = useId();
  const saveId = id ?? generatedId;
  const [externalSrc, setExternalSrc] = useState<string | null>(null);
  const useExternalLink = externalSrc === src;
  const resolveUrl = useCallback(async () => src, [src]);
  // 保存ダイアログの前に本文を確認する。CORS で読めないときはダイアログを出さずに
  // リンクへ切り替え、Content-Length があれば保存の上限判定に渡す。
  const prepare = useCallback(
    async (signal: AbortSignal): Promise<DesktopSavePrepared> => {
      let response: Response;
      try {
        response = await fetch(src, { credentials: "omit", signal });
      } catch (error) {
        throw new AttachmentDownloadError("network", { cause: error });
      }
      if (!response.ok) {
        void response.body?.cancel().catch(() => {});
        throw new AttachmentDownloadError("http", { status: response.status });
      }
      const raw = response.headers.get("content-length");
      const parsed = raw ? Number(raw) : Number.NaN;
      const size = Number.isFinite(parsed) && parsed > 0 ? Math.floor(parsed) : undefined;
      // 保存先のダイアログの間は本文を読まない。接続を止めて、転送は保存時に開き直す。
      await response.body?.cancel().catch(() => {});
      return { size };
    },
    [src],
  );
  const { state, pending, available, download, cancel } = useDesktopSave({
    id: saveId,
    fileName: name,
    source: { kind: "url", resolveUrl, size, prepare },
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
        setExternalSrc(src);
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
