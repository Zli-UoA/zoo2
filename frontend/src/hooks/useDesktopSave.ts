import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { AttachmentDownloadError, type AttachmentUrlResolver } from "../lib/api/attachments";
import {
  DesktopDownloadDestinationError,
  DesktopDownloadLimitError,
  desktopDownloadSettings,
  hasDesktopBridge,
} from "../lib/desktop";
import {
  DesktopDownloadCancelledError,
  cancelDesktopDownload,
  desktopDownloadsAvailable,
  downloadBlobToDesktop,
  downloadUrlToDesktop,
  useDesktopDownloadState,
} from "../lib/desktopDownloads";

/** 保存ダイアログを出す前に確認した本文と大きさ。 */
export type DesktopSavePrepared = { response?: Response; size?: number | null };

/** 保存する本文の出所。表示に使った URL か、復号済みの `Blob` のどちらか。 */
export type DesktopSaveSource =
  | {
      kind: "url";
      resolveUrl: AttachmentUrlResolver;
      size?: number | null;
      /** 保存ダイアログの前に本文を確認し、開いた応答をそのまま転送に使う。 */
      prepare?: () => Promise<DesktopSavePrepared>;
    }
  | {
      kind: "blob";
      load: (signal: AbortSignal, report: (fraction: number | null) => void) => Promise<Blob>;
      size?: number | null;
    };

export type DesktopSaveErrorDescription = {
  message?: string;
  description?: string;
};

/**
 * デスクトップ版で保存できるか。`null` は Tauri のブリッジはあるが判定が済んでいない状態で、
 * その間は保存ボタンを出す（押したときに判定し直し、失敗を伝える）。
 */
function useDesktopDownloadsAvailable(): boolean | null {
  const [available, setAvailable] = useState<boolean | null>(() =>
    hasDesktopBridge() ? null : false,
  );
  useEffect(() => {
    if (!hasDesktopBridge()) return;
    let active = true;
    void desktopDownloadsAvailable().then(
      (value) => {
        if (active) setAvailable(value);
      },
      () => {
        // 一時的な判定失敗ではブラウザーの保存へ切り替えない。押したときにもう一度判定する。
        if (active) setAvailable(null);
      },
    );
    return () => {
      active = false;
    };
  }, []);
  return available;
}

/**
 * 添付とプレビューの保存で共通の判定・進捗・中止・結果の通知。
 * 本文の取得は呼び出し側が `source` で渡し、ブラウザーの保存のときだけ `fallback` を使う。
 */
export function useDesktopSave({
  id,
  fileName,
  source,
  fallback,
  onFailure,
  describeError,
}: {
  id: string;
  fileName: string;
  source: DesktopSaveSource;
  fallback: () => Promise<void>;
  onFailure?: (error: unknown) => void;
  describeError?: (error: unknown) => DesktopSaveErrorDescription | undefined;
}) {
  const { t } = useTranslation();
  const state = useDesktopDownloadState(id);
  const available = useDesktopDownloadsAvailable();
  const [pending, setPending] = useState(false);
  const running = useRef(false);
  const current = useRef({ source, fallback, onFailure, describeError });
  useEffect(() => {
    current.current = { source, fallback, onFailure, describeError };
  }, [source, fallback, onFailure, describeError]);

  function reportFailure(error: unknown) {
    const { onFailure: notify, describeError: describe } = current.current;
    if (error instanceof DesktopDownloadDestinationError) {
      void desktopDownloadSettings()
        .then((settings) => {
          toast.error(
            settings?.ask_every_time === false
              ? t("attachments.downloadDestinationUnsupportedAskEveryTimeOff")
              : t("attachments.downloadDestinationUnsupported"),
          );
        })
        .catch(() => toast.error(t("attachments.downloadDestinationUnsupported")));
    } else if (error instanceof DesktopDownloadLimitError) {
      toast.error(t("attachments.downloadTooMany"));
    } else {
      const described = describe?.(error);
      const description =
        described?.description ??
        (error instanceof AttachmentDownloadError && error.reason === "http" && error.status
          ? `HTTP ${error.status}`
          : undefined);
      toast.error(
        described?.message ?? t("attachments.downloadError"),
        description ? { description } : undefined,
      );
    }
    notify?.(error);
  }

  async function download() {
    if (running.current || state) return;
    running.current = true;
    setPending(true);
    try {
      // 古い版のアプリではコマンドがない。Web と同じくブラウザーの保存に戻す。
      if (!(await desktopDownloadsAvailable())) {
        await current.current.fallback();
        return;
      }
      const { source: active } = current.current;
      let response: Response | undefined;
      let size = active.size;
      if (active.kind === "url" && active.prepare) {
        const prepared = await active.prepare();
        response = prepared.response;
        if (prepared.size != null) size = prepared.size;
      }
      const savedName =
        active.kind === "url"
          ? await downloadUrlToDesktop(id, fileName, active.resolveUrl, { size, response })
          : await downloadBlobToDesktop(id, fileName, active.load, size);
      // 同じ名前が既にあったときは番号が足されるので、実際の名前を伝える。
      if (savedName) toast.success(t("attachments.downloadSaved", { name: savedName }));
    } catch (error) {
      if (!(error instanceof DesktopDownloadCancelledError)) reportFailure(error);
    } finally {
      running.current = false;
      setPending(false);
    }
  }

  return {
    state,
    available,
    pending,
    download,
    cancel: () => cancelDesktopDownload(id),
  };
}
