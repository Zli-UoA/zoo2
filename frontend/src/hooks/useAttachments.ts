import { useLiveQuery } from "@tanstack/react-db";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { createConfiguredApiClient } from "../lib/api/client";
import { uploadAttachment, type UploadedAttachment } from "../lib/api/attachments";
import type { AttachmentUrlResponse } from "../lib/api/types";
import { useCollections } from "../lib/electric/collections-context";

const ATTACHMENT_URL_TTL_MS = 4 * 60 * 1000;
const urlCache = new Map<string, { url: string; expiresAt: number }>();

type AttachmentUrlStatus = "loading" | "ready" | "error";

export interface AttachmentUrlState {
  url: string | null;
  status: AttachmentUrlStatus;
  reload: () => void;
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
  const [status, setStatus] = useState<AttachmentUrlStatus>(() =>
    urlCache.has(attachmentId) ? "ready" : "loading",
  );
  const [url, setUrl] = useState<string | null>(() => urlCache.get(attachmentId)?.url ?? null);
  const requestId = useRef(0);

  const load = useCallback(
    (useCache: boolean) => {
      const cached = urlCache.get(attachmentId);
      if (useCache && cached && cached.expiresAt > Date.now()) {
        setStatus("ready");
        setUrl(cached.url);
        return;
      }
      const current = ++requestId.current;
      setStatus("loading");
      client
        .requestWithTx<AttachmentUrlResponse>(`/api/attachments/${attachmentId}/url`, {
          method: "GET",
        })
        .then(({ data }) => {
          if (current !== requestId.current) {
            return;
          }
          urlCache.set(attachmentId, {
            url: data.download_url,
            expiresAt: Date.now() + ATTACHMENT_URL_TTL_MS,
          });
          setUrl(data.download_url);
          setStatus("ready");
        })
        .catch(() => {
          if (current !== requestId.current) {
            return;
          }
          setUrl(null);
          setStatus("error");
        });
    },
    [attachmentId, client],
  );

  useEffect(() => {
    load(true);
    return () => {
      requestId.current += 1;
    };
  }, [load]);

  const reload = useCallback(() => {
    urlCache.delete(attachmentId);
    load(false);
  }, [attachmentId, load]);

  return { url, status, reload };
}
