import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ApiError, createConfiguredApiClient } from "../../lib/api/client";
import { useCollections } from "../../lib/electric/collections-context";
import { Button } from "../ui/button";
import { SettingsSection } from "../ui/control-panel";

type Settings = { channel_ids: string[]; channels: { id: string; channel_name: string }[] };
export function DefaultChannelsSettings() {
  const { t } = useTranslation();
  const { config } = useCollections();
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  const selectionToRestore = useRef<{ client: typeof client; ids: string[] } | null>(null);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [selected, setSelected] = useState<string[]>([]);
  const [revision, setRevision] = useState(0);
  const [error, setError] = useState(false);
  const [pending, setPending] = useState(false);
  const [saved, setSaved] = useState(false);
  const [refreshed, setRefreshed] = useState(false);
  useEffect(() => {
    const controller = new AbortController();
    setError(false);
    setSettings(null);
    void client
      .get<Settings>("/api/workspace/default-channels", { signal: controller.signal })
      .then((value) => {
        if (controller.signal.aborted) return;
        setSettings(value);
        const previous = selectionToRestore.current;
        const ids = previous?.client === client ? previous.ids : value.channel_ids;
        selectionToRestore.current = null;
        setSelected(ids.filter((id) => value.channels.some((channel) => channel.id === id)));
      })
      .catch(() => {
        if (!controller.signal.aborted) setError(true);
      });
    return () => controller.abort();
  }, [client, revision]);
  return (
    <SettingsSection
      title={t("defaultChannels.title")}
      description={t("defaultChannels.description")}
    >
      <p className="text-xs text-muted-foreground">{t("defaultChannels.limit")}</p>
      {refreshed && settings ? <p role="alert">{t("defaultChannels.refreshed")}</p> : null}
      {error ? <p role="alert">{t("common.writeError")}</p> : null}
      {!settings ? (
        error ? (
          <Button onClick={() => setRevision((value) => value + 1)}>
            {t("attachments.retry")}
          </Button>
        ) : (
          <p role="status">{t("common.loading")}</p>
        )
      ) : (
        <form
          className="space-y-3"
          onSubmit={async (event) => {
            event.preventDefault();
            if (pending) return;
            setRefreshed(false);
            if (selected.length > 100) {
              setError(true);
              return;
            }
            setPending(true);
            setError(false);
            setSaved(false);
            try {
              await client.request("/api/workspace/default-channels", {
                method: "PATCH",
                body: { channel_ids: selected },
              });
              setSaved(true);
            } catch (cause) {
              if (
                cause instanceof ApiError &&
                cause.status === 400 &&
                typeof cause.body === "object" &&
                cause.body !== null &&
                "code" in cause.body &&
                cause.body.code === "default_channel_unavailable"
              ) {
                selectionToRestore.current = { client, ids: selected };
                setSettings(null);
                setRefreshed(true);
                setRevision((value) => value + 1);
              } else {
                setError(true);
              }
            } finally {
              setPending(false);
            }
          }}
        >
          <fieldset disabled={pending} className="max-h-72 space-y-2 overflow-y-auto">
            <legend className="sr-only">{t("defaultChannels.title")}</legend>
            {settings.channels.length === 0 ? (
              <p>{t("defaultChannels.empty")}</p>
            ) : (
              settings.channels.map((channel) => (
                <label key={channel.id} className="flex min-w-0 items-center gap-2 text-sm">
                  <input
                    type="checkbox"
                    disabled={selected.length >= 100 && !selected.includes(channel.id)}
                    checked={selected.includes(channel.id)}
                    onChange={(event) => {
                      setSaved(false);
                      setSelected((current) =>
                        event.target.checked
                          ? [...current, channel.id]
                          : current.filter((id) => id !== channel.id),
                      );
                    }}
                  />
                  <span className="break-all">#{channel.channel_name}</span>
                </label>
              ))
            )}
          </fieldset>
          <Button type="submit" disabled={pending}>
            {t("common.save")}
          </Button>
          {saved ? <p role="status">{t("defaultChannels.saved")}</p> : null}
        </form>
      )}
    </SettingsSection>
  );
}
