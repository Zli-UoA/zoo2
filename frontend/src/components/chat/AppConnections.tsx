import { Input } from "../ui/input";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { useCurrentUser } from "../../hooks/useCurrentUser";
import { createConfiguredApiClient } from "../../lib/api/client";
import { useCollections } from "../../lib/electric/collections-context";
import { useShell } from "../layout/shell-context";
import { inputClassName } from "../ui/input-styles";
import { Button } from "../ui/button";

type Session = { id: string; app_id: string; title: string; state: string };
export function AppConnections({
  appId,
  channelId,
  connections,
  assistant,
}: {
  appId: string;
  channelId?: string | null;
  connections: boolean;
  assistant: boolean;
}) {
  const { t } = useTranslation();
  const { config } = useCollections();
  const { currentUserId } = useCurrentUser();
  const shell = useShell();
  const selectedChannelId = channelId === undefined ? shell.selectedChannelId : channelId;
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  const [connection, setConnection] = useState<string | null>(null);
  const [sessions, setSessions] = useState<Session[]>([]);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [revision, setRevision] = useState(0);
  useEffect(() => {
    if (!connections && !assistant) return;
    const controller = new AbortController();
    const load = () => {
      if (connections)
        void client
          .get<{ state: string }>(`/api/apps/${appId}/connection`, { signal: controller.signal })
          .then((v) => {
            if (!controller.signal.aborted) setConnection(v.state);
          })
          .catch(() => {
            if (!controller.signal.aborted) setError(t("apps.loadError"));
          });
      if (assistant)
        void client
          .get<Session[]>("/api/assistant-sessions", { signal: controller.signal })
          .then((v) => {
            if (!controller.signal.aborted) setSessions(v.filter((s) => s.app_id === appId));
          })
          .catch(() => {
            if (!controller.signal.aborted) setError(t("apps.loadError"));
          });
    };
    load();
    const timer = setInterval(load, 2000);
    return () => {
      controller.abort();
      clearInterval(timer);
    };
  }, [appId, connections, assistant, client, revision, t]);
  useEffect(() => {
    if (!assistant || !selectedChannelId) return;
    const controller = new AbortController();
    void client
      .request(`/api/apps/${appId}/context`, {
        method: "POST",
        body: {
          channel_id: selectedChannelId,
          context: { view: "app", channel_id: selectedChannelId },
        },
        signal: controller.signal,
      })
      .catch(() => {});
    return () => controller.abort();
  }, [appId, assistant, selectedChannelId, client]);
  async function run(path: string, body: unknown, method = "POST") {
    setBusy(true);
    setError("");
    try {
      await client.request(path, { method, body });
      setRevision((v) => v + 1);
    } catch {
      setError(t("apps.actionError"));
    } finally {
      setBusy(false);
    }
  }
  if (!connections && !assistant) return null;
  return (
    <div className="space-y-3 rounded-lg border p-3">
      {connections ? (
        <div className="flex flex-wrap items-center justify-between gap-2 text-sm">
          <span>
            {connection === null
              ? t("apps.loading")
              : t(
                  `apps.userConnection.${connection === "connected" ? "connected" : connection === "requested" ? "requested" : "disconnected"}`,
                )}
          </span>
          <Button
            variant="outline"
            disabled={busy || connection === null}
            onClick={() =>
              void run(`/api/apps/${appId}/connection`, { connect: connection === "disconnected" })
            }
          >
            {t(
              connection === "disconnected"
                ? "apps.userConnection.connect"
                : "apps.userConnection.disconnect",
            )}
          </Button>
        </div>
      ) : null}
      {assistant && selectedChannelId ? (
        <Button
          variant="outline"
          disabled={busy || !currentUserId}
          onClick={() =>
            void run("/api/assistant-sessions", {
              app_id: appId,
              user_id: currentUserId,
              channel_id: selectedChannelId,
              title: t("apps.assistant.session"),
              context: { channel_id: selectedChannelId },
            })
          }
        >
          {t("apps.assistant.start")}
        </Button>
      ) : null}
      {assistant
        ? sessions
            .filter((s) => s.state === "running")
            .map((s) => (
              <div key={s.id} className="flex items-center gap-2 rounded-lg bg-muted/40 p-2">
                <Input
                  className={`${inputClassName()} min-w-0 flex-1`}
                  disabled={busy}
                  aria-label={t("apps.assistant.title")}
                  maxLength={200}
                  defaultValue={s.title}
                  onBlur={(e) => {
                    if (e.target.value !== s.title)
                      void run(
                        `/api/assistant-sessions/${s.id}`,
                        { title: e.target.value },
                        "PATCH",
                      );
                  }}
                />
                <Button
                  variant="ghost"
                  disabled={busy}
                  onClick={() =>
                    void run(`/api/assistant-sessions/${s.id}`, { state: "stopped" }, "PATCH")
                  }
                >
                  {t("apps.assistant.stop")}
                </Button>
              </div>
            ))
        : null}
      {error ? (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      ) : null}
    </div>
  );
}
