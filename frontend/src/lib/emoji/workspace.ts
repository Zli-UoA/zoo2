import { useCallback, useEffect, useMemo, useState } from "react";
import { createConfiguredApiClient } from "../api/client";
import { joinUrl, resolvePublicConfig, type AppConfig } from "../config";
import { useOptionalCollections } from "../electric/collections-context";

export interface WorkspaceEmoji {
  width?: number | null;
  height?: number | null;
  name: string;
  url: string;
  category: string;
  aliases: string[];
}
const requests = new WeakMap<AppConfig, Promise<WorkspaceEmoji[]>>();

export function useWorkspaceEmojis(enabled = true) {
  const collections = useOptionalCollections();
  const config = useMemo(() => collections?.config ?? resolvePublicConfig(), [collections]);
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  const [emojis, setEmojis] = useState<WorkspaceEmoji[]>([]);
  const [loading, setLoading] = useState(true);
  const [failed, setFailed] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const retry = useCallback(() => setAttempt((n) => n + 1), []);
  useEffect(() => {
    if (!enabled) {
      setLoading(false);
      return;
    }
    const controller = new AbortController();
    setLoading(true);
    setFailed(false);
    let request = requests.get(config);
    if (!request) {
      request = client.get<WorkspaceEmoji[]>("/api/emojis").finally(() => requests.delete(config));
      requests.set(config, request);
    }
    void request
      .then((values) => {
        if (!controller.signal.aborted) setEmojis(values);
      })
      .catch(() => {
        if (!controller.signal.aborted) setFailed(true);
      })
      .finally(() => {
        if (!controller.signal.aborted) setLoading(false);
      });
    const refresh = () => retry();
    window.addEventListener("workspace-emojis-changed", refresh);
    return () => {
      controller.abort();
      window.removeEventListener("workspace-emojis-changed", refresh);
    };
  }, [client, config, attempt, retry, enabled]);
  return {
    emojis,
    loading,
    failed,
    retry,
    imageUrl: (url: string) => (/^\/(?!\/)/.test(url) ? joinUrl(config.apiBaseUrl, url) : url),
  };
}
