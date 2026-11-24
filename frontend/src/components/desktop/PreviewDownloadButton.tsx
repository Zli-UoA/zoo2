import { useCallback, useId, useState } from "react";
import { useTranslation } from "react-i18next";

import { useDesktopSave, type DesktopSavePrepared } from "../../hooks/useDesktopSave";
import {
  AttachmentDownloadError,
  downloadAttachmentWithRetry,
  type AttachmentUrlResolver,
} from "../../lib/api/attachments";
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
  resolveUrl,
}: {
  /** 同じファイルを表示する複数の場所で進捗と中止を共有するための識別子。 */
  id?: string;
  name: string;
  src: string;
  size?: number | null;
  showLabel?: boolean;
  className?: string;
  /** 表示に使う URL を更新できる経路。署名付き URL の期限切れに備えて渡す。 */
  resolveUrl?: AttachmentUrlResolver;
}) {
  const { t } = useTranslation();
  const generatedId = useId();
  const saveId = id ?? generatedId;
  const [externalSrc, setExternalSrc] = useState<string | null>(null);
  const useExternalLink = externalSrc === src;
  const staticUrl = useCallback(async () => src, [src]);
  const urlResolver = resolveUrl ?? staticUrl;
  // 保存ダイアログの前に本文を確認する。CORS で読めないときはダイアログを出さずに
  // リンクへ切り替え、Content-Length があれば保存の上限判定に渡す。
  const prepare = useCallback(
    async (signal: AbortSignal): Promise<DesktopSavePrepared> => {
      const url = await urlResolver({ refresh: false });
      let response: Response;
      try {
        response = await fetch(url, { credentials: "omit", signal });
      } catch (error) {
        throw new AttachmentDownloadError("network", { cause: error });
      }
      if (!response.ok) {
        void response.body?.cancel().catch(() => {});
        throw new AttachmentDownloadError("http", { status: response.status });
      }
      const encoding = response.headers.get("content-encoding");
      const raw = response.headers.get("content-length");
      const parsed = raw ? Number(raw) : Number.NaN;
      // Content-Encoding がある場合、Content-Length は圧縮後のサイズであり、
      // 展開後の本体サイズとは一致しないことがある。Rust 側の保存上限に誤った値を渡さない。
      const size =
        !encoding || encoding === "identity"
          ? Number.isFinite(parsed) && parsed > 0
            ? Math.floor(parsed)
            : undefined
          : undefined;
      // 保存先のダイアログの間は本文を読まない。接続を止めて、転送は保存時に開き直す。
      await response.body?.cancel().catch(() => {});
      return { size };
    },
    [urlResolver],
  );
  const { state, pending, available, download, cancel } = useDesktopSave({
    id: saveId,
    fileName: name,
    source: { kind: "url", resolveUrl: urlResolver, size, prepare },
    fallback: async () => {
      await downloadAttachmentWithRetry(urlResolver, name);
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
