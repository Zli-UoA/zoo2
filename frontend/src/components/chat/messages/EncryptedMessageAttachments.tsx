import type { AttachmentInfo } from "@zoo/crypto-wasm";
import { LockIcon, PaperclipIcon } from "lucide-react";
import { createContext, useContext, useEffect, useLayoutEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { forgetAttachmentUrl, resolveAttachmentUrl } from "../../../lib/api/attachmentUrls";
import { createConfiguredApiClient } from "../../../lib/api/client";
import { formatAttachmentSize, mediaPreviewKind } from "../../../lib/attachments";
import { DesktopDownloadDestinationError, DesktopDownloadLimitError, desktopDownloadSettings } from "../../../lib/desktop";
import {
  DesktopDownloadCancelledError,
  cancelDesktopDownload,
  desktopDownloadsAvailable,
  downloadBlobToDesktop,
  useDesktopDownloadState,
} from "../../../lib/desktopDownloads";
import {
  EncryptedAttachmentError,
  fetchEncryptedAttachment,
  receivedAttachmentView,
  workerAttachmentCrypto,
} from "../../../lib/e2ee/attachments";
import {
  decryptedAttachmentCacheFor,
  decryptedAttachmentCacheKey,
} from "../../../lib/e2ee/decryptedAttachmentCache";
import { useE2ee } from "../../../lib/e2ee/e2ee-context";
import { useCollections } from "../../../lib/electric/collections-context";
import type { AttachmentRow } from "../../../lib/electric/types";
import { DownloadProgressButton } from "../../desktop/DownloadProgressButton";
import { useLocale } from "../../i18n/LocaleProvider";
import { MediaPreview } from "../../ui/media-preview";
import { SensitiveMedia } from "../../ui/sensitive-media";

/** 本文の `AttachmentInfo` から、復号した中身を返す。 */
export type EncryptedAttachmentLoader = (info: AttachmentInfo) => Promise<Blob>;

/** Storybook とテストで、暗号の Worker の代わりの読み込みを渡す。 */
export const EncryptedAttachmentLoaderContext = createContext<EncryptedAttachmentLoader | null>(
  null,
);

/**
 * 復号した添付の取り出し。`peek` と `failed` はキャッシュだけを見る（再マウントでスピナーを出さず、
 * 復号できなかった添付を取り直さない）。
 */
export interface EncryptedAttachmentSource {
  peek(info: AttachmentInfo): Blob | undefined;
  failed(info: AttachmentInfo): boolean;
  load(
    info: AttachmentInfo,
    options?: {
      retry?: boolean;
      /** 取得を中止する。復号の途中でも、取得が終わった時点で呼び出し側が確かめる。 */
      signal?: AbortSignal;
      /** 暗号文を読んだ量（平文の大きさではない）。全体が分からなければ `total` は null。 */
      onProgress?: (received: number, total: number | null) => void;
    },
  ): Promise<Blob>;
}

/**
 * 暗号の Worker と API で、暗号化した添付を取得して復号する。解錠していなければ失敗する。
 *
 * 復号した `Blob` は Worker ごとのキャッシュ（`decryptedAttachmentCacheFor`）に、署名付き URL は接続の設定ごとの
 * キャッシュ（`lib/api/attachmentUrls.ts`）に残す。メッセージ一覧の行が再マウントされても、取り直さない。
 */
export function useEncryptedAttachmentSource(): EncryptedAttachmentSource {
  const override = useContext(EncryptedAttachmentLoaderContext);
  const { worker } = useE2ee();
  const { config } = useCollections();
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  return useMemo(() => {
    const scope = override ?? worker;
    const cache = scope ? decryptedAttachmentCacheFor(scope) : null;
    const loadFromNetwork = async (
      info: AttachmentInfo,
      options: {
        signal?: AbortSignal;
        onProgress?: (received: number, total: number | null) => void;
      } = {},
    ): Promise<Blob> => {
      if (override) {
        options.onProgress?.(0, null);
        const blob = await override(info);
        options.onProgress?.(blob.size, blob.size);
        return blob;
      }
      if (!worker) throw new EncryptedAttachmentError("decrypt");
      try {
        return await fetchEncryptedAttachment(
          client,
          info,
          workerAttachmentCrypto(worker),
          fetch,
          (attachmentId) => resolveAttachmentUrl(client, config, attachmentId),
          options,
        );
      } catch (error) {
        // 期限切れなどで取得できなかった URL は、次に試すときに取り直す。中止は失敗ではないので残す。
        if (
          error instanceof EncryptedAttachmentError &&
          error.reason === "download" &&
          !options.signal?.aborted
        ) {
          forgetAttachmentUrl(config, info.attachment_id);
        }
        throw error;
      }
    };
    return {
      peek: (info) => cache?.peek(info),
      failed: (info) => cache?.failed(info) ?? false,
      load: (info, options = {}) => {
        const { retry, ...progress } = options;
        const loadOne = () => loadFromNetwork(info, progress);
        return cache
          ? cache.load(info, loadOne, { retry, isolated: progress.signal !== undefined })
          : loadOne();
      },
    };
  }, [override, worker, client, config]);
}

type DecryptedState = { status: "loading" } | { status: "ready"; blob: Blob } | { status: "error" };

/** 添付を復号する。キャッシュにあればすぐに返す。 */
function useDecryptedAttachment(info: AttachmentInfo) {
  const source = useEncryptedAttachmentSource();
  const key = decryptedAttachmentCacheKey(info);
  const [state, setState] = useState<DecryptedState>(() => {
    const cached = source.peek(info);
    if (cached) return { status: "ready", blob: cached };
    return source.failed(info) ? { status: "error" } : { status: "loading" };
  });
  const [attempt, setAttempt] = useState(0);

  useEffect(() => {
    const cached = source.peek(info);
    if (cached) {
      setState({ status: "ready", blob: cached });
      return;
    }
    // 復号できなかった添付は、再試行を押すまで取り直さない。
    if (attempt === 0 && source.failed(info)) {
      setState({ status: "error" });
      return;
    }
    let cancelled = false;
    setState({ status: "loading" });
    source.load(info, { retry: attempt > 0 }).then(
      (blob) => {
        if (!cancelled) setState({ status: "ready", blob });
      },
      () => {
        if (!cancelled) setState({ status: "error" });
      },
    );
    return () => {
      cancelled = true;
    };
    // `info` は描画のたびに作り直されることがあるので、中身（キー）が変わったときだけ取り直す。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [source, key, attempt]);

  const blob = state.status === "ready" ? state.blob : null;
  const url = useObjectUrl(blob);
  return { state, url, retry: () => setAttempt((n) => n + 1) };
}

/**
 * `Blob` の object URL。コンポーネントごとに作り、外れたら捨てる。描画の前に作るので、キャッシュにある
 * 画像はスピナーを挟まずに出る。
 */
function useObjectUrl(blob: Blob | null): string | null {
  const [url, setUrl] = useState<string | null>(null);
  useLayoutEffect(() => {
    if (!blob) {
      setUrl(null);
      return;
    }
    const objectUrl = URL.createObjectURL(blob);
    setUrl(objectUrl);
    return () => URL.revokeObjectURL(objectUrl);
  }, [blob]);
  return url;
}

/**
 * 暗号化チャンネルのメッセージの添付（#364）。復号した本文の `attachments` だけを表示し、
 * サーバーの添付の行（`encrypted.bin`）は使わない。
 */
export function EncryptedMessageAttachments({
  attachments,
  serverAttachments = [],
}: {
  attachments: AttachmentInfo[];
  /**
   * サーバーの添付の行（`encrypted.bin`）。中身には使わず、NSFW のチャンネルでサーバーが付けるぼかしの印
   * （`is_sensitive`）だけを読む。
   */
  serverAttachments?: AttachmentRow[];
}) {
  if (attachments.length === 0) return null;
  const serverSensitive = new Set(
    serverAttachments.filter((row) => row.is_sensitive).map((row) => row.id),
  );
  const shown = attachments.map((attachment) => {
    const view = receivedAttachmentView(attachment);
    return {
      info: { ...attachment, name: view.name, mimetype: view.mimetype },
      blocked: view.blocked,
      sensitive: Boolean(attachment.sensitive) || serverSensitive.has(attachment.attachment_id),
    };
  });
  const previews = shown.filter(
    ({ info, blocked }) => !blocked && mediaPreviewKind(info.mimetype, info.name),
  );
  const files = shown.filter(
    ({ info, blocked }) => blocked || !mediaPreviewKind(info.mimetype, info.name),
  );
  return (
    <div className="mt-2 flex max-w-xl flex-col gap-2">
      {previews.length > 0 ? (
        <div className="grid w-full grid-cols-1 gap-2">
          {previews.map(({ info, sensitive }) => (
            <SensitiveMedia
              key={`${info.attachment_id}:${sensitive}`}
              sensitive={sensitive}
              preview={{ width: info.width ?? null, height: info.height ?? null }}
              className="w-full"
            >
              <EncryptedPreview attachment={info} />
            </SensitiveMedia>
          ))}
        </div>
      ) : null}
      {files.length > 0 ? (
        <ul className="flex flex-col gap-1.5">
          {files.map(({ info, blocked }) => (
            <li key={info.attachment_id}>
              <EncryptedFileCard attachment={info} blocked={blocked} />
            </li>
          ))}
        </ul>
      ) : null}
    </div>
  );
}

function EncryptedPreview({ attachment }: { attachment: AttachmentInfo }) {
  const { state, url, retry } = useDecryptedAttachment(attachment);
  if (state.status === "error") {
    return <EncryptedFileCard attachment={attachment} failed onRetry={retry} />;
  }
  return (
    <MediaPreview
      src={url}
      name={attachment.name}
      contentType={attachment.mimetype}
      width={attachment.width ?? null}
      height={attachment.height ?? null}
      loading={state.status === "loading"}
      onRetry={retry}
      footer={
        state.status === "ready" ? (
          <EncryptedDownloadButton attachment={attachment} blob={state.blob} showLabel />
        ) : undefined
      }
    />
  );
}

function EncryptedFileCard({
  attachment,
  failed = false,
  blocked = false,
  onRetry,
}: {
  attachment: AttachmentInfo;
  failed?: boolean;
  /** 平文の添付ならサーバーが拒否する種類（実行ファイルなど）。取得も保存もさせない。 */
  blocked?: boolean;
  onRetry?: () => void;
}) {
  const { t } = useTranslation();
  const { locale } = useLocale();
  return (
    <div className="flex items-center gap-2 rounded-lg border bg-muted/30 px-2.5 py-2">
      <PaperclipIcon className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
      <div className="min-w-0 flex-1">
        <p className="truncate text-sm font-medium">{attachment.name}</p>
        <p className="flex items-center gap-1 truncate text-xs text-muted-foreground">
          <LockIcon className="size-3 shrink-0" aria-label={t("e2ee.attachments.encrypted")} />
          {formatAttachmentSize(attachment.size, locale)}
          {failed ? ` · ${t("e2ee.attachments.decryptFailed")}` : null}
          {blocked ? ` · ${t("e2ee.attachments.blocked")}` : null}
        </p>
      </div>
      {failed && onRetry ? (
        <button
          type="button"
          className="text-xs font-medium text-link hover:underline"
          onClick={onRetry}
        >
          {t("attachments.retry")}
        </button>
      ) : null}
      {failed || blocked ? null : <EncryptedDownloadButton attachment={attachment} />}
    </div>
  );
}

function EncryptedDownloadButton({
  attachment,
  blob,
  showLabel = false,
}: {
  attachment: AttachmentInfo;
  blob?: Blob;
  showLabel?: boolean;
}) {
  const { t } = useTranslation();
  const source = useEncryptedAttachmentSource();
  const [downloading, setDownloading] = useState(false);
  const desktopState = useDesktopDownloadState(attachment.attachment_id);

  function reportFailure(error: unknown) {
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
      return;
    }
    if (error instanceof DesktopDownloadLimitError) {
      // 同時保存の上限。ほかの保存が終われば再試行できる。
      toast.error(t("attachments.downloadTooMany"));
      return;
    }
    toast.error(
      error instanceof EncryptedAttachmentError && error.reason === "decrypt"
        ? t("e2ee.attachments.decryptFailed")
        : t("attachments.downloadError"),
    );
  }

  async function download() {
    if (downloading || desktopState) return;
    setDownloading(true);
    try {
      // デスクトップ版では OS の保存ダイアログと保存先の設定を使う。復号した `Blob` を同じ書き込み先へ渡す。
      if (await desktopDownloadsAvailable()) {
        await downloadBlobToDesktop(
          attachment.attachment_id,
          attachment.name,
          async (signal, report) => {
            if (blob) {
              report(1);
              return blob;
            }
            // 押したことを、利用者の明示的な再試行として扱う（覚えている復号の失敗も取り直す）。
            return source.load(attachment, {
              retry: true,
              signal,
              onProgress: (received, total) =>
                report(total && total > 0 ? Math.min(1, received / total) : null),
            });
          },
          attachment.size,
        );
      } else {
        // 押したことを、利用者の明示的な再試行として扱う（覚えている復号の失敗も取り直す）。
        saveBlob(blob ?? (await source.load(attachment, { retry: true })), attachment.name);
      }
    } catch (error) {
      if (!(error instanceof DesktopDownloadCancelledError)) reportFailure(error);
    } finally {
      setDownloading(false);
    }
  }

  return (
    <DownloadProgressButton
      state={desktopState}
      pending={downloading}
      showLabel={showLabel}
      downloadLabel={t("attachments.download", { name: attachment.name })}
      downloadShortLabel={t("attachments.downloadShort")}
      cancelLabel={t("attachments.cancelDownload", { name: attachment.name })}
      cancelShortLabel={t("attachments.cancelDownloadShort")}
      finishingLabel={t("attachments.saving")}
      progressLabel={t("attachments.downloading")}
      onDownload={() => void download()}
      onCancel={() => cancelDesktopDownload(attachment.attachment_id)}
    />
  );
}

function saveBlob(blob: Blob, name: string) {
  const objectUrl = URL.createObjectURL(blob);
  try {
    const anchor = document.createElement("a");
    anchor.href = objectUrl;
    anchor.download = name;
    anchor.rel = "noopener";
    document.body.append(anchor);
    anchor.click();
    anchor.remove();
  } finally {
    window.setTimeout(() => URL.revokeObjectURL(objectUrl), 1_000);
  }
}
