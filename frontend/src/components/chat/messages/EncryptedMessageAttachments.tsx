import type { AttachmentInfo } from "@zoo/crypto-wasm";
import { DownloadIcon, LoaderCircleIcon, LockIcon, PaperclipIcon } from "lucide-react";
import { createContext, useContext, useEffect, useLayoutEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { forgetAttachmentUrl, resolveAttachmentUrl } from "../../../lib/api/attachmentUrls";
import { createConfiguredApiClient } from "../../../lib/api/client";
import { formatAttachmentSize, isInlineImage } from "../../../lib/attachments";
import {
  EncryptedAttachmentError,
  fetchEncryptedAttachment,
  workerAttachmentCrypto,
} from "../../../lib/e2ee/attachments";
import {
  decryptedAttachmentCacheFor,
  decryptedAttachmentCacheKey,
} from "../../../lib/e2ee/decryptedAttachmentCache";
import { useE2ee } from "../../../lib/e2ee/e2ee-context";
import { useCollections } from "../../../lib/electric/collections-context";
import { useLocale } from "../../i18n/LocaleProvider";
import { Button } from "../../ui/button";
import { Dialog, DialogContent, DialogTitle } from "../../ui/dialog";
import { SensitiveMedia } from "../../ui/sensitive-media";
import { Spinner } from "../../ui/spinner";

/** 本文の `AttachmentInfo` から、復号した中身を返す。 */
export type EncryptedAttachmentLoader = (info: AttachmentInfo) => Promise<Blob>;

/** Storybook とテストで、暗号の Worker の代わりの読み込みを渡す。 */
export const EncryptedAttachmentLoaderContext = createContext<EncryptedAttachmentLoader | null>(
  null,
);

/** 復号した添付の取り出し。`peek` はキャッシュだけを見る（再マウントでスピナーを出さない）。 */
export interface EncryptedAttachmentSource {
  peek(info: AttachmentInfo): Blob | undefined;
  load(info: AttachmentInfo): Promise<Blob>;
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
    const loader: EncryptedAttachmentLoader = override
      ? override
      : async (info) => {
          if (!worker) throw new EncryptedAttachmentError("decrypt");
          try {
            return await fetchEncryptedAttachment(
              client,
              info,
              workerAttachmentCrypto(worker),
              fetch,
              (attachmentId) => resolveAttachmentUrl(client, config, attachmentId),
            );
          } catch (error) {
            // 期限切れなどで取得できなかった URL は、次に試すときに取り直す。
            if (error instanceof EncryptedAttachmentError && error.reason === "download") {
              forgetAttachmentUrl(config, info.attachment_id);
            }
            throw error;
          }
        };
    return {
      peek: (info) => cache?.peek(info),
      load: (info) => (cache ? cache.load(info, loader) : loader(info)),
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
    return cached ? { status: "ready", blob: cached } : { status: "loading" };
  });
  const [attempt, setAttempt] = useState(0);

  useEffect(() => {
    const cached = source.peek(info);
    if (cached) {
      setState({ status: "ready", blob: cached });
      return;
    }
    let cancelled = false;
    setState({ status: "loading" });
    source.load(info).then(
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
export function EncryptedMessageAttachments({ attachments }: { attachments: AttachmentInfo[] }) {
  if (attachments.length === 0) return null;
  const images = attachments.filter((attachment) => isInlineImage(attachment.mimetype));
  const files = attachments.filter((attachment) => !isInlineImage(attachment.mimetype));
  return (
    <div className="mt-2 flex max-w-xl flex-col gap-2">
      {images.length > 0 ? (
        <div
          className={
            images.length === 1 ? "w-fit max-w-full" : "grid w-full max-w-sm grid-cols-2 gap-2"
          }
        >
          {images.map((attachment) => (
            <SensitiveMedia
              key={attachment.attachment_id}
              sensitive={attachment.sensitive}
              preview={{ width: attachment.width ?? null, height: attachment.height ?? null }}
              className={images.length === 1 ? "w-64 max-w-full" : "w-full"}
            >
              <EncryptedImage attachment={attachment} multiple={images.length > 1} />
            </SensitiveMedia>
          ))}
        </div>
      ) : null}
      {files.length > 0 ? (
        <ul className="flex flex-col gap-1.5">
          {files.map((attachment) => (
            <li key={attachment.attachment_id}>
              <EncryptedFileCard attachment={attachment} />
            </li>
          ))}
        </ul>
      ) : null}
    </div>
  );
}

function EncryptedImage({
  attachment,
  multiple,
}: {
  attachment: AttachmentInfo;
  multiple: boolean;
}) {
  const { t } = useTranslation();
  const { state, url, retry } = useDecryptedAttachment(attachment);
  const [open, setOpen] = useState(false);
  const aspectRatio =
    attachment.width && attachment.height ? `${attachment.width} / ${attachment.height}` : "4 / 3";

  if (state.status === "error") {
    return <EncryptedFileCard attachment={attachment} failed onRetry={retry} />;
  }
  return (
    <>
      <div
        className={
          multiple
            ? "aspect-square w-full overflow-hidden rounded-lg border bg-muted"
            : "w-64 max-w-full overflow-hidden rounded-lg border bg-muted"
        }
        style={!multiple ? { aspectRatio, minHeight: 80, maxHeight: 384 } : undefined}
      >
        {state.status === "loading" || !url ? (
          <Spinner label={t("attachments.loading")} className="size-full" />
        ) : (
          <button
            type="button"
            aria-label={t("attachments.preview", { name: attachment.name })}
            className="block size-full cursor-zoom-in outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
            onClick={() => setOpen(true)}
          >
            <img
              src={url}
              alt={attachment.name}
              className={
                multiple
                  ? "size-full object-cover transition-opacity hover:opacity-90"
                  : "size-full object-contain transition-opacity hover:opacity-90"
              }
            />
          </button>
        )}
      </div>
      {state.status === "ready" && url ? (
        <Dialog open={open} onOpenChange={setOpen}>
          <DialogContent className="w-auto max-w-[calc(100vw-2rem)] p-2 sm:max-w-[min(96vw,80rem)]">
            <DialogTitle className="sr-only">{attachment.name}</DialogTitle>
            <img
              src={url}
              alt={attachment.name}
              className="mx-auto max-h-[78vh] max-w-full object-contain"
            />
            <div className="flex items-center justify-between gap-3 px-1">
              <span className="min-w-0 truncate text-xs text-muted-foreground">
                {attachment.name}
              </span>
              <EncryptedDownloadButton attachment={attachment} blob={state.blob} showLabel />
            </div>
          </DialogContent>
        </Dialog>
      ) : null}
    </>
  );
}

function EncryptedFileCard({
  attachment,
  failed = false,
  onRetry,
}: {
  attachment: AttachmentInfo;
  failed?: boolean;
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
      {failed ? null : <EncryptedDownloadButton attachment={attachment} />}
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

  async function download() {
    if (downloading) return;
    setDownloading(true);
    try {
      saveBlob(blob ?? (await source.load(attachment)), attachment.name);
    } catch (error) {
      toast.error(
        error instanceof EncryptedAttachmentError && error.reason === "decrypt"
          ? t("e2ee.attachments.decryptFailed")
          : t("attachments.downloadError"),
      );
    } finally {
      setDownloading(false);
    }
  }

  const label = t("attachments.download", { name: attachment.name });
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
