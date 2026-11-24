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
  prepareDesktopDownload,
  useDesktopDownloadState,
} from "../lib/desktopDownloads";

/** 保存ダイアログを出す前に確認した本文の大きさ。 */
export type DesktopSavePrepared = { size?: number | null };

/** 保存する本文の出所。表示に使った URL か、復号済みの `Blob` のどちらか。 */
export type DesktopSaveSource =
  | {
      kind: "url";
      resolveUrl: AttachmentUrlResolver;
      size?: number | null;
      /** 保存ダイアログの前に CORS と本文の大きさを確認する。 */
      prepare?: (signal: AbortSignal) => Promise<DesktopSavePrepared>;
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

  function reportFailure(
    error: unknown,
    callbacks: Pick<typeof current.current, "onFailure" | "describeError"> = current.current,
  ) {
    const { onFailure: notify, describeError: describe } = callbacks;
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
    // src が保存中に変わっても、失敗通知は保存開始時の対象へ適用する。
    const started = current.current;
    try {
      // 古い版のアプリではコマンドがない。Web と同じくブラウザーの保存に戻す。
      if (!(await desktopDownloadsAvailable())) {
        await started.fallback();
        return;
      }
      const { source: active } = started;
      let size = active.size;
      let controller: AbortController | undefined;
      if (active.kind === "url" && active.prepare) {
        const { prepare } = active;
        // 確認の待機中も円を中止に切り替え、同じ controller で転送まで続ける。
        const prepared = await prepareDesktopDownload(id, (signal) => prepare(signal));
        // 別の保存が先に始まっていたときは、状態を壊さず何もしない。
        if (!prepared) return;
        controller = prepared.controller;
        if (prepared.value.size != null) size = prepared.value.size;
      }
      const savedName =
        active.kind === "url"
          ? await downloadUrlToDesktop(id, fileName, active.resolveUrl, { size, controller })
          : await downloadBlobToDesktop(id, fileName, active.load, size);
      // 同じ名前が既にあったときは番号が足されるので、実際の名前を伝える。
      if (savedName) toast.success(t("attachments.downloadSaved", { name: savedName }));
    } catch (error) {
      if (!(error instanceof DesktopDownloadCancelledError)) reportFailure(error, started);
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
