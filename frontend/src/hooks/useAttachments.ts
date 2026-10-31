import { useLiveQuery } from "@tanstack/react-db";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { attachmentUrlExpiresAt, shouldRefreshAttachmentUrl } from "../lib/attachments";
import { createConfiguredApiClient } from "../lib/api/client";
import { uploadAttachment, type UploadedAttachment } from "../lib/api/attachments";
import type { AttachmentUrlResponse } from "../lib/api/types";
import { useCollections } from "../lib/electric/collections-context";

const urlCache = new Map<string, { url: string; expiresAt: number }>();

type AttachmentUrlStatus = "loading" | "ready" | "error";

export interface AttachmentUrlState {
  url: string | null;
  status: AttachmentUrlStatus;
  expiresAt: number | null;
  reload: () => void;
  getFreshUrl: () => Promise<string | null>;
}

export function useMessageAttachments(channelId: string | null) {
  const { attachments } = useCollections();
  return useLiveQuery((q) =>
    channelId ? q.from({ attachments: attachments(channelId) }) : undefined,
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

export function useAttachmentUrl(attachmentId: string): AttachmentUrlState {
  const { config } = useCollections();
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  const cached = urlCache.get(attachmentId);
  const [status, setStatus] = useState<AttachmentUrlStatus>(cached ? "ready" : "loading");
  const [url, setUrl] = useState<string | null>(cached?.url ?? null);
  const [expiresAt, setExpiresAt] = useState<number | null>(cached?.expiresAt ?? null);
  const requestId = useRef(0);

  const load = useCallback(
    async (useCache: boolean): Promise<string | null> => {
      const cachedEntry = urlCache.get(attachmentId);
      if (useCache && cachedEntry && !shouldRefreshAttachmentUrl(cachedEntry.expiresAt)) {
        setUrl(cachedEntry.url);
        setExpiresAt(cachedEntry.expiresAt);
        setStatus("ready");
        return cachedEntry.url;
      }
      const current = ++requestId.current;
      setStatus("loading");
      try {
        const { data } = await client.requestWithTx<AttachmentUrlResponse>(
          `/api/attachments/${attachmentId}/url`,
          { method: "GET" },
        );
        if (current !== requestId.current) {
          return null;
        }
        const nextExpiresAt = attachmentUrlExpiresAt(data.download_url);
        urlCache.set(attachmentId, { url: data.download_url, expiresAt: nextExpiresAt });
        setUrl(data.download_url);
        setExpiresAt(nextExpiresAt);
        setStatus("ready");
        return data.download_url;
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
    [attachmentId, client],
  );

  useEffect(() => {
    void load(true);
    return () => {
      requestId.current += 1;
    };
  }, [load]);

  const reload = useCallback(() => {
    urlCache.delete(attachmentId);
    void load(false);
  }, [attachmentId, load]);

  const getFreshUrl = useCallback(() => {
    const cachedEntry = urlCache.get(attachmentId);
    if (cachedEntry && !shouldRefreshAttachmentUrl(cachedEntry.expiresAt)) {
      return Promise.resolve(cachedEntry.url);
    }
    return load(false);
  }, [attachmentId, load]);

  return { url, status, expiresAt, reload, getFreshUrl };
}
