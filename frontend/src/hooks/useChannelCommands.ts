import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import { useTranslation } from "react-i18next";
import { createConfiguredApiClient } from "../lib/api/client";
import { useCollections } from "../lib/electric/collections-context";
import type { CommandSuggestion } from "../components/chat/messages/composerSuggestions";

/** Refresh on opening slash completion, not on every keystroke. */
export function useChannelCommands(channelId: string, userId: string | null) {
  const { config } = useCollections();
  const { t } = useTranslation();
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  const request = useRef<AbortController | null>(null);
  const key = `${userId}:${channelId}`;
  const [result, setResult] = useState<{ key: string; commands: CommandSuggestion[] }>();
  useEffect(() => () => request.current?.abort(), [client, key]);
  const load = useCallback(() => {
    if (!userId) return;
    request.current?.abort();
    const controller = new AbortController();
    request.current = controller;
    client
      .get<CommandSuggestion[]>(`/api/apps/commands?channel_id=${encodeURIComponent(channelId)}`, {
        signal: controller.signal,
      })
      .then((commands) => {
        if (!controller.signal.aborted) setResult({ key, commands });
      })
      .catch(() => {
        if (!controller.signal.aborted) {
          setResult({ key, commands: [] });
          toast.error(t("apps.loadError"));
        }
      });
  }, [client, channelId, key, userId, t]);
  return { commands: result?.key === key ? result.commands : [], load };
}
