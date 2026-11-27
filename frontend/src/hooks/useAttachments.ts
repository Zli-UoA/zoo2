import { useLiveQuery } from "@tanstack/react-db";
import type { AttachmentInfo } from "@zoo/crypto-wasm";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { shouldRefreshAttachmentUrl } from "../lib/attachments";
import {
  cachedAttachmentUrl,
  forgetAttachmentUrl,
  requestAttachmentUrl,
} from "../lib/api/attachmentUrls";
import { createConfiguredApiClient } from "../lib/api/client";
import {
  AttachmentUploadError,
  attachmentUrlError,
  uploadAttachment,
  type UploadedAttachment,
} from "../lib/api/attachments";
import { uploadEncryptedAttachment, workerAttachmentCrypto } from "../lib/e2ee/attachments";
import { useE2ee } from "../lib/e2ee/e2ee-context";
import { useCollections } from "../lib/electric/collections-context";

type AttachmentUrlStatus = "loading" | "ready" | "error";

export interface AttachmentUrlState {
  url: string | null;
  thumbnailUrl?: string;
  status: AttachmentUrlStatus;
  expiresAt: number | null;
  reload: () => void;
  /**
   * ダウンロードなど、表示の状態に左右されずに URL が必要なときに使う。
   * `refresh` を渡すとキャッシュを捨てて取り直す（ダウンロードの再試行用）。
   * 取得に失敗したときは、状態コードを保った `AttachmentDownloadError` を投げる。
   */
  getFreshUrl: (options?: { refresh?: boolean }) => Promise<string>;
}

export function useMessageAttachments(channelId: string | null, enabled = true) {
  const { attachments } = useCollections();
  return useLiveQuery((q) =>
    channelId && enabled ? q.from({ attachments: attachments(channelId) }) : undefined,
  );
}

export function useAttachmentUpload(channelId: string) {
  const { config } = useCollections();
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  return useCallback(
    (file: File): Promise<UploadedAttachment> => uploadAttachment(client, channelId, file),
    [client, channelId],
  );
}

/**
 * 暗号化チャンネルの添付（#364）。ファイルを暗号の Worker で暗号化して上げ、Megolm で暗号化する本文の
 * `attachments` に入れる `AttachmentInfo` を返す。解錠していなければ失敗する。
 */
export function useEncryptedAttachmentUpload(channelId: string) {
  const { config } = useCollections();
  const { worker } = useE2ee();
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  return useCallback(
    async (file: File, options: { sensitive?: boolean } = {}): Promise<AttachmentInfo> => {
      if (!worker) throw new AttachmentUploadError("request");
      return uploadEncryptedAttachment(client, channelId, file, workerAttachmentCrypto(worker), {
        sensitive: options.sensitive,
      });
    },
    [client, channelId, worker],
  );
}

export function useAttachmentUrl(attachmentId: string): AttachmentUrlState {
  const { config } = useCollections();
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  const cached = cachedAttachmentUrl(config, attachmentId);
  const [status, setStatus] = useState<AttachmentUrlStatus>(cached ? "ready" : "loading");
  const [url, setUrl] = useState<string | null>(cached?.url ?? null);
  const [thumbnailUrl, setThumbnailUrl] = useState(cached?.thumbnailUrl);
  const [expiresAt, setExpiresAt] = useState<number | null>(cached?.expiresAt ?? null);
  const requestId = useRef(0);
  // 再試行の判定には、クリック時点のクロージャではなく、その時点で表示している URL の期限を使う。
  const visible = useRef({ url: cached?.url ?? null, expiresAt: cached?.expiresAt ?? null });
  const show = useCallback(
    (nextUrl: string | null, nextExpiresAt: number | null, nextThumbnail?: string) => {
      visible.current = { url: nextUrl, expiresAt: nextExpiresAt };
      setUrl(nextUrl);
      setThumbnailUrl(nextThumbnail);
      setExpiresAt(nextExpiresAt);
    },
    [],
  );
  // 表示中の URL が実際の期限前で、まだ取得に使えるか。更新マージンは含めない。
  const hasUsableUrl = useCallback(
    () =>
      visible.current.url !== null &&
      visible.current.expiresAt !== null &&
      visible.current.expiresAt > Date.now(),
    [],
  );

  const load = useCallback(
    async (useCache: boolean): Promise<string | null> => {
      const cachedEntry = cachedAttachmentUrl(config, attachmentId);
      if (useCache && cachedEntry && !shouldRefreshAttachmentUrl(cachedEntry.expiresAt)) {
        show(cachedEntry.url, cachedEntry.expiresAt, cachedEntry.thumbnailUrl);
        setStatus("ready");
        return cachedEntry.url;
      }
      const current = ++requestId.current;
      setStatus("loading");
      try {
        const value = await requestAttachmentUrl(client, config, attachmentId);
        if (current !== requestId.current) return null;
        show(value.url, value.expiresAt, value.thumbnailUrl);
        setStatus("ready");
        return value.url;
      } catch {
        if (current !== requestId.current) {
          return null;
        }
        show(null, null);
        setStatus("error");
        return null;
      }
    },
    [attachmentId, client, config, show],
  );

  useEffect(() => {
    void load(true);
    return () => {
      requestId.current += 1;
    };
  }, [load]);

  const reload = useCallback(() => {
    forgetAttachmentUrl(config, attachmentId);
    void load(false);
  }, [attachmentId, load, config]);

  // 表示用の `load()` は `requestId` で古い応答を捨てるが、ダウンロードは利用者の操作なので
  // 取得できた URL を必ず返す。行の再マウントや仮想スクロールで `load()` の結果が捨てられても、
  // 押したダウンロードが失敗しないようにする（#640）。
  // 表示状態の更新だけは、config の切替やアンマウントをまたいだ古い応答で上書きしないよう
  // 同じ `requestId` で守る。
  const getFreshUrl = useCallback(
    async (options: { refresh?: boolean } = {}): Promise<string> => {
      const current = requestId.current;
      if (options.refresh) {
        forgetAttachmentUrl(config, attachmentId);
      } else {
        const cachedEntry = cachedAttachmentUrl(config, attachmentId);
        if (cachedEntry && !shouldRefreshAttachmentUrl(cachedEntry.expiresAt)) {
          if (current === requestId.current) {
            show(cachedEntry.url, cachedEntry.expiresAt, cachedEntry.thumbnailUrl);
            setStatus("ready");
          }
          return cachedEntry.url;
        }
      }
      // 表示中の URL がまだ実際に使える間は、取得のやり直しでスケルトンに戻さない。
      if (!hasUsableUrl() && current === requestId.current) {
        setStatus("loading");
      }
      try {
        const value = await requestAttachmentUrl(client, config, attachmentId);
        if (current === requestId.current) {
          show(value.url, value.expiresAt, value.thumbnailUrl);
          setStatus("ready");
        }
        return value.url;
      } catch (error) {
        if (current === requestId.current) {
          // ダウンロードの再試行が失敗しても、まだ使える URL を表示しているプレビューは壊さない。
          // 期限切れの URL だけは `ready` のまま残さず、読み込み直せるようにする。
          // 1回目の取得で表示 URL が更新されている場合は、その新しい期限で判定する。
          // 取得開始の判定と違い、ここでは更新マージンではなく実際の期限だけを見る。
          if (!hasUsableUrl()) {
            show(null, null);
            setStatus("error");
          }
        }
        throw attachmentUrlError(error);
      }
    },
    [attachmentId, client, config, hasUsableUrl, show],
  );

  return { url, thumbnailUrl, status, expiresAt, reload, getFreshUrl };
}
