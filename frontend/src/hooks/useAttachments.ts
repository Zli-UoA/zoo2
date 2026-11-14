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
  uploadAttachment,
  type UploadedAttachment,
} from "../lib/api/attachments";
import { uploadEncryptedAttachment, workerAttachmentCrypto } from "../lib/e2ee/attachments";
import { useE2ee } from "../lib/e2ee/e2ee-context";
import { useCollections } from "../lib/electric/collections-context";

type AttachmentUrlStatus = "loading" | "ready" | "error";

export interface AttachmentUrlState {
  url: string | null;
  status: AttachmentUrlStatus;
  expiresAt: number | null;
  reload: () => void;
  getFreshUrl: () => Promise<string | null>;
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
  const [expiresAt, setExpiresAt] = useState<number | null>(cached?.expiresAt ?? null);
  const requestId = useRef(0);

  const load = useCallback(
    async (useCache: boolean): Promise<string | null> => {
      const cachedEntry = cachedAttachmentUrl(config, attachmentId);
      if (useCache && cachedEntry && !shouldRefreshAttachmentUrl(cachedEntry.expiresAt)) {
        setUrl(cachedEntry.url);
        setExpiresAt(cachedEntry.expiresAt);
        setStatus("ready");
        return cachedEntry.url;
      }
      const current = ++requestId.current;
      setStatus("loading");
      try {
        const value = await requestAttachmentUrl(client, config, attachmentId);
        if (current !== requestId.current) return null;
        const nextExpiresAt = value.expiresAt;
        setUrl(value.url);
        setExpiresAt(nextExpiresAt);
        setStatus("ready");
        return value.url;
      } catch {
        if (current !== requestId.current) {
          return null;
        }
        setUrl(null);
        setExpiresAt(null);
        setStatus("error");
        return null;
      }
    },
    [attachmentId, client, config],
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

  const getFreshUrl = useCallback(() => {
    const cachedEntry = cachedAttachmentUrl(config, attachmentId);
    if (cachedEntry && !shouldRefreshAttachmentUrl(cachedEntry.expiresAt)) {
      return Promise.resolve(cachedEntry.url);
    }
    return load(false);
  }, [attachmentId, load, config]);

  return { url, status, expiresAt, reload, getFreshUrl };
}
