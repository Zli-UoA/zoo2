import { useLiveQuery } from "@tanstack/react-db";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { attachmentUrlExpiresAt, shouldRefreshAttachmentUrl } from "../lib/attachments";
import { createConfiguredApiClient } from "../lib/api/client";
import { uploadAttachment, type UploadedAttachment } from "../lib/api/attachments";
import type { AttachmentUrlResponse } from "../lib/api/types";
import { useCollections } from "../lib/electric/collections-context";

type UrlEntry = { url: string; expiresAt: number };
const urlCaches = new WeakMap<object, Map<string, UrlEntry>>();
const urlRequests = new WeakMap<object, Map<string, Promise<UrlEntry>>>();

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

export function useAttachmentUrl(attachmentId: string): AttachmentUrlState {
  const { config } = useCollections();
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  const urlCache = useMemo(() => {
    let cache = urlCaches.get(config);
    if (!cache) urlCaches.set(config, (cache = new Map()));
    return cache;
  }, [config]);
  const requests = useMemo(() => {
    let pending = urlRequests.get(config);
    if (!pending) urlRequests.set(config, (pending = new Map()));
    return pending;
  }, [config]);
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
        let pending = requests.get(attachmentId);
        if (!pending) {
          pending = client
            .requestWithTx<AttachmentUrlResponse>(`/api/attachments/${attachmentId}/url`, {
              method: "GET",
            })
            .then(({ data }) => {
              const value = {
                url: data.download_url,
                expiresAt: attachmentUrlExpiresAt(data.download_url),
              };
              urlCache.set(attachmentId, value);
              return value;
            })
            .finally(() => requests.delete(attachmentId));
          requests.set(attachmentId, pending);
        }
        const value = await pending;
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
    [attachmentId, client, urlCache, requests],
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
  }, [attachmentId, load, urlCache]);

  const getFreshUrl = useCallback(() => {
    const cachedEntry = urlCache.get(attachmentId);
    if (cachedEntry && !shouldRefreshAttachmentUrl(cachedEntry.expiresAt)) {
      return Promise.resolve(cachedEntry.url);
    }
    return load(false);
  }, [attachmentId, load, urlCache]);

  return { url, status, expiresAt, reload, getFreshUrl };
}
