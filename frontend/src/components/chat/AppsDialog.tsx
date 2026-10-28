import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { BotIcon } from "lucide-react";
import { createConfiguredApiClient } from "../../lib/api/client";
import { useCollections } from "../../lib/electric/collections-context";
import { useUsers } from "../../hooks/useRealtime";
import { useCurrentUser } from "../../hooks/useCurrentUser";
import { useShell } from "../layout/shell-context";
import { Button } from "../ui/button";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "../ui/dialog";
import { inputClassName } from "../ui/input-styles";
import { AppConnections } from "./AppConnections";
import { AppBlocks } from "./AppBlocks";
import { AppAvatar } from "./AppIdentity";

type Manifest = {
  scopes?: string[];
  user_connections?: boolean;
  transport?: string;
  event_url?: string | null;
  notification_only?: boolean;
  dm_enabled?: boolean;
  commands?: string[];
  shortcuts?: string[];
  message_shortcuts?: string[];
};
type App = {
  id: string;
  name: string;
  description: string;
  icon_url: string | null;
  bot_user_id: string;
  owner_id: string;
  revision: number;
  status: "draft" | "pending" | "installed" | "disabled";
  manifest: Manifest;
};
type Surface = {
  id: string;
  kind: string;
  channel_id: string | null;
  document: { blocks?: unknown };
};
const defaultManifest = JSON.stringify(
  {
    scopes: ["messages:read", "messages:write", "dm:write", "ui:write"],
    events: ["message.created", "app.mentioned"],
    transport: "websocket",
    dm_enabled: true,
  },
  null,
  2,
);
export function AppsButton() {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [appId, setAppId] = useState<string>();
  const [messageId, setMessageId] = useState<string>();
  useEffect(() => {
    const listener = (event: Event) => {
      setMessageId(undefined);
      setAppId((event as CustomEvent<string>).detail);
      setOpen(true);
    };
    const messageListener = (event: Event) => {
      setAppId(undefined);
      setMessageId((event as CustomEvent<string>).detail);
      setOpen(true);
    };
    window.addEventListener("zoo:open-app", listener);
    window.addEventListener("zoo:message-actions", messageListener);
    return () => {
      window.removeEventListener("zoo:open-app", listener);
      window.removeEventListener("zoo:message-actions", messageListener);
    };
  }, []);
  return (
    <>
      <Button
        variant="ghost"
        size="icon-sm"
        aria-label={t("apps.title")}
        title={t("apps.title")}
        onClick={() => {
          setMessageId(undefined);
          setAppId(undefined);
          setOpen(true);
        }}
      >
        <BotIcon />
      </Button>
      {open ? (
        <AppsDialog initialId={appId} messageId={messageId} onClose={() => setOpen(false)} />
      ) : null}
    </>
  );
}
export function AppsDialog({
  initialId,
  messageId,
  onClose,
}: {
  initialId?: string;
  messageId?: string;
  onClose: () => void;
}) {
  const { currentUserId } = useCurrentUser();
  return currentUserId ? (
    <UserAppsDialog
      key={currentUserId}
      initialId={initialId}
      messageId={messageId}
      onClose={onClose}
    />
  ) : null;
}
function UserAppsDialog({
  initialId,
  messageId,
  onClose,
}: {
  initialId?: string;
  messageId?: string;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const { config } = useCollections();
  const { currentUserId, isAdmin } = useCurrentUser();
  const shell = useShell();
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  const [apps, setApps] = useState<App[]>([]);
  const [selected, setSelected] = useState(initialId);
  const [revision, setRevision] = useState(0);
  const [busy, setBusy] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [editing, setEditing] = useState(false);
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [icon, setIcon] = useState("");
  const [manifest, setManifest] = useState(defaultManifest);
  const [credential, setCredential] = useState("");
  const [commandArguments, setCommandArguments] = useState<Record<string, string>>({});
  const [surfaces, setSurfaces] = useState<Surface[]>([]);
  const [deliveries, setDeliveries] = useState<
    {
      id: string;
      type: string;
      attempts: number;
      acked_at: string | null;
      last_error: string | null;
    }[]
  >([]);
  const people = useUsers().data ?? [];
  const app = apps.find((a) => a.id === selected);
  const canManage = app && (app.owner_id === currentUserId || isAdmin);
  useEffect(() => {
    const controller = new AbortController();
    setLoading(true);
    client
      .get<App[]>("/api/apps", { signal: controller.signal })
      .then(async (list) => {
        if (initialId && !list.some((a) => a.id === initialId)) {
          try {
            const detail = await client.get<App>(`/api/apps/${encodeURIComponent(initialId)}`, {
              signal: controller.signal,
            });
            list = [...list, detail];
          } catch {
            /* Unpublished apps remain private. */
          }
        }
        if (!controller.signal.aborted) setApps(list);
      })
      .catch(() => {
        if (!controller.signal.aborted) setError(t("apps.loadError"));
      })
      .finally(() => {
        if (!controller.signal.aborted) setLoading(false);
      });
    return () => controller.abort();
  }, [client, revision, t, initialId]);
  useEffect(() => {
    setCredential("");
    setDeliveries([]);
  }, [selected]);
  useEffect(() => {
    if (!selected || app?.status !== "installed") {
      setSurfaces([]);
      return;
    }
    const controller = new AbortController();
    const load = () =>
      client
        .get<Surface[]>(`/api/apps/${selected}/surfaces`, { signal: controller.signal })
        .then(setSurfaces)
        .catch(() => {});
    void load();
    const timer = window.setInterval(load, 2000);
    return () => {
      controller.abort();
      window.clearInterval(timer);
      setSurfaces([]);
    };
  }, [client, selected, app?.status]);
  async function run(fn: () => Promise<unknown>) {
    if (busy) return;
    setBusy(true);
    setError("");
    try {
      await fn();
      setRevision((v) => v + 1);
    } catch {
      setError(t("apps.actionError"));
    } finally {
      setBusy(false);
    }
  }
  async function action(kind: string) {
    await run(async () => {
      const result = await client.request<Record<string, unknown>>(
        `/api/apps/${selected}/actions/${kind}`,
        { method: "POST", body: kind === "install" ? { revision: app?.revision } : undefined },
      );
      if (kind === "token" || kind === "connection") setCredential(JSON.stringify(result, null, 2));
    });
  }
  function edit(existing?: App) {
    setName(existing?.name ?? "");
    setDescription(existing?.description ?? "");
    setIcon(existing?.icon_url ?? "");
    setManifest(existing ? JSON.stringify(existing.manifest, null, 2) : defaultManifest);
    setEditing(true);
  }
  async function save() {
    await run(async () => {
      const result = await client.request<{ id: string }>(
        selected ? `/api/apps/${selected}` : "/api/apps",
        {
          method: selected ? "PATCH" : "POST",
          body: {
            name,
            description,
            icon_url: icon || null,
            manifest: JSON.parse(manifest) as unknown,
          },
        },
      );
      setSelected(result.id);
      setEditing(false);
    });
  }
  async function invoke(kind: string, command?: string) {
    await run(() =>
      client.request(`/api/apps/${selected}/invoke`, {
        method: "POST",
        body: {
          kind,
          name: command,
          values: kind === "command" ? { text: commandArguments[command ?? ""] ?? "" } : {},
          message_id: kind === "shortcut" ? messageId : undefined,
          channel_id: kind === "home" ? null : shell.selectedChannelId,
        },
      }),
    );
  }
  async function dm() {
    if (!app) return;
    await run(async () => {
      const result = await client.post<{ id: string }>("/api/dms", {
        target_user_ids: [app.bot_user_id],
      });
      window.location.assign(`/channels/${result.data.id}`);
    });
  }
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      <DialogContent className="max-h-[90dvh] overflow-y-auto sm:max-w-3xl">
        <DialogHeader>
          <DialogTitle>{t("apps.title")}</DialogTitle>
          <DialogDescription>{t("apps.description")}</DialogDescription>
        </DialogHeader>
        {error ? (
          <p role="alert" className="text-sm text-destructive">
            {error}
          </p>
        ) : null}
        <div className="grid gap-4 sm:grid-cols-[12rem_1fr]">
          <div className="space-y-2">
            <Button
              variant="outline"
              disabled={busy}
              onClick={() => {
                setSelected(undefined);
                edit();
              }}
            >
              {t("apps.create")}
            </Button>
            {loading ? <p role="status">{t("apps.loading")}</p> : null}
            {apps
              .filter((a) => !messageId || a.manifest.message_shortcuts?.length)
              .map((a) => (
                <button
                  key={a.id}
                  className={`flex w-full items-center gap-2 rounded-lg p-2 text-left text-sm ${selected === a.id ? "bg-accent" : "hover:bg-muted"}`}
                  onClick={() => {
                    setSelected(a.id);
                    setEditing(false);
                    setError("");
                  }}
                >
                  <AppAvatar label={a.name} isBot src={a.icon_url} size="sm" />
                  <span className="min-w-0 truncate">{a.name}</span>
                </button>
              ))}
          </div>
          <div className="min-w-0 space-y-4">
            {editing ? (
              <form
                className="space-y-3"
                onSubmit={(e) => {
                  e.preventDefault();
                  void save();
                }}
              >
                <label className="flex flex-col gap-1 text-sm">
                  {t("apps.name")}
                  <input
                    className={inputClassName()}
                    required
                    maxLength={80}
                    value={name}
                    onChange={(e) => setName(e.target.value)}
                  />
                </label>
                <label className="flex flex-col gap-1 text-sm">
                  {t("apps.about")}
                  <textarea
                    className={inputClassName()}
                    maxLength={4000}
                    value={description}
                    onChange={(e) => setDescription(e.target.value)}
                  />
                </label>
                <label className="flex flex-col gap-1 text-sm">
                  {t("apps.icon")}
                  <input
                    className={inputClassName()}
                    type="url"
                    value={icon}
                    onChange={(e) => setIcon(e.target.value)}
                  />
                </label>
                <label className="flex flex-col gap-1 text-sm">
                  {t("apps.manifest")}
                  <textarea
                    className={`${inputClassName()} min-h-60 font-mono text-xs`}
                    value={manifest}
                    onChange={(e) => setManifest(e.target.value)}
                  />
                </label>
                <Button type="submit" disabled={busy}>
                  {t("apps.save")}
                </Button>
              </form>
            ) : app ? (
              <>
                <div className="flex items-center gap-3">
                  <AppAvatar label={app.name} isBot src={app.icon_url} />
                  <div>
                    <h3 className="font-semibold">
                      {app.name} <span className="text-xs text-muted-foreground">APP</span>
                    </h3>
                    <p className="text-xs text-muted-foreground">
                      {t(`apps.status.${app.status}`)}
                    </p>
                  </div>
                </div>
                <p className="whitespace-pre-wrap text-sm">{app.description}</p>
                {app.status === "installed" ? (
                  <AppConnections
                    appId={app.id}
                    connections={Boolean(app.manifest.user_connections)}
                    assistant={Boolean(app.manifest.scopes?.includes("assistant:write"))}
                  />
                ) : null}
                <p className="break-all text-xs text-muted-foreground">
                  {t("apps.developer")}:{" "}
                  {people.find((user) => user.id === app.owner_id)?.display_name ||
                    t("apps.unknownDeveloper")}
                </p>
                <div className="flex flex-wrap gap-2">
                  {app.status === "installed" &&
                  app.manifest.dm_enabled &&
                  !app.manifest.notification_only ? (
                    <Button disabled={busy} onClick={() => void dm()}>
                      {t("apps.message")}
                    </Button>
                  ) : null}
                  {app.status === "installed" &&
                  !app.manifest.notification_only &&
                  shell.selectedChannelId ? (
                    <Button
                      variant="outline"
                      disabled={busy}
                      onClick={() =>
                        void run(() =>
                          client.request(
                            `/api/apps/${app.id}/channels/${shell.selectedChannelId}`,
                            { method: "POST" },
                          ),
                        )
                      }
                    >
                      {t("apps.addChannel")}
                    </Button>
                  ) : null}
                  {canManage ? (
                    <>
                      <Button variant="outline" disabled={busy} onClick={() => edit(app)}>
                        {t("apps.edit")}
                      </Button>
                      <Button
                        variant="outline"
                        disabled={busy}
                        onClick={() => void action("submit")}
                      >
                        {t("apps.requestApproval")}
                      </Button>
                      {isAdmin && app.status === "pending" ? (
                        <Button disabled={busy} onClick={() => void action("install")}>
                          {t("apps.approve")}
                        </Button>
                      ) : null}
                      {app.status === "installed" ? (
                        <>
                          <Button
                            variant="outline"
                            disabled={busy}
                            onClick={() => void action("token")}
                          >
                            {t("apps.issueToken")}
                          </Button>
                          <Button
                            variant="outline"
                            disabled={busy}
                            onClick={() => void action("connection")}
                          >
                            {t("apps.connection")}
                          </Button>
                        </>
                      ) : null}
                      <Button
                        variant="destructive"
                        disabled={busy}
                        onClick={() => void action("disable")}
                      >
                        {t("apps.disable")}
                      </Button>
                    </>
                  ) : null}
                </div>
                {credential ? (
                  <label className="flex flex-col gap-1 text-xs">
                    {t("apps.credential")}
                    <input
                      className={`${inputClassName()} font-mono`}
                      type="password"
                      readOnly
                      value={credential}
                      autoComplete="off"
                    />
                    <Button
                      variant="outline"
                      size="sm"
                      onClick={() =>
                        void navigator.clipboard
                          .writeText(credential)
                          .catch(() => setError(t("apps.actionError")))
                      }
                    >
                      {t("apps.copy")}
                    </Button>
                  </label>
                ) : null}
                <Button
                  variant="ghost"
                  disabled={busy}
                  onClick={() =>
                    void run(() =>
                      client.request(`/api/apps/${app.id}/authorization`, { method: "DELETE" }),
                    )
                  }
                >
                  {t("apps.revokeConsent")}
                </Button>
                <div className="space-y-1">
                  {canManage && app.manifest.transport ? (
                    <p className="text-xs text-muted-foreground">
                      {t("apps.transport")}: {app.manifest.transport}
                    </p>
                  ) : null}
                  {canManage && app.manifest.event_url ? (
                    <p className="break-all text-xs text-muted-foreground">
                      {t("apps.eventEndpoint")}: {app.manifest.event_url}
                    </p>
                  ) : null}
                  <h4 className="text-sm font-medium">{t("apps.permissions")}</h4>
                  <ul className="list-inside list-disc text-xs text-muted-foreground">
                    {app.manifest.scopes?.map((scope) => (
                      <li key={scope}>{scope}</li>
                    ))}
                  </ul>
                </div>
                {app.manifest.commands?.map((command) => (
                  <div key={command} className="flex items-end gap-2">
                    <label className="flex flex-1 flex-col gap-1 text-sm">
                      /{command} {t("apps.commandArguments")}
                      <input
                        className={inputClassName()}
                        value={commandArguments[command] ?? ""}
                        maxLength={4000}
                        onChange={(e) =>
                          setCommandArguments((previous) => ({
                            ...previous,
                            [command]: e.target.value,
                          }))
                        }
                      />
                    </label>
                    <Button
                      variant="outline"
                      disabled={busy || !shell.selectedChannelId}
                      onClick={() => void invoke("command", command)}
                    >
                      /{command}
                    </Button>
                  </div>
                ))}
                {(messageId ? app.manifest.message_shortcuts : app.manifest.shortcuts)?.map(
                  (shortcut) => (
                    <Button
                      key={shortcut}
                      variant="outline"
                      disabled={busy}
                      onClick={() => void invoke("shortcut", shortcut)}
                    >
                      {shortcut}
                    </Button>
                  ),
                )}
                {app.status === "installed" ? (
                  <Button variant="ghost" disabled={busy} onClick={() => void invoke("home")}>
                    {t("apps.openHome")}
                  </Button>
                ) : null}
                {surfaces
                  .filter((surface) => surface.kind === "home")
                  .map((surface) => (
                    <section
                      key={surface.id}
                      className="rounded-lg border p-3"
                      aria-label={t(surface.kind === "modal" ? "apps.form" : "apps.home")}
                    >
                      <AppBlocks
                        blocks={surface.document.blocks}
                        appId={app.id}
                        surfaceId={surface.id}
                        channelId={surface.channel_id}
                        submit={surface.kind === "modal"}
                      />
                    </section>
                  ))}
                {canManage ? (
                  <>
                    <Button
                      variant="ghost"
                      onClick={() =>
                        void run(async () =>
                          setDeliveries(await client.get(`/api/apps/${app.id}/deliveries`)),
                        )
                      }
                    >
                      {t("apps.deliveries")}
                    </Button>
                    {deliveries.map((delivery) => (
                      <div key={delivery.id} className="rounded border p-2 text-xs">
                        <p>
                          {delivery.type} · {delivery.attempts} ·{" "}
                          {delivery.acked_at
                            ? t("apps.delivered")
                            : (delivery.last_error ?? t("apps.pending"))}
                        </p>
                        {!delivery.acked_at ? (
                          <Button
                            variant="ghost"
                            size="sm"
                            disabled={busy}
                            onClick={() =>
                              void run(() =>
                                client.request(
                                  `/api/apps/${app.id}/deliveries/${delivery.id}/retry`,
                                  { method: "POST" },
                                ),
                              )
                            }
                          >
                            {t("apps.retry")}
                          </Button>
                        ) : null}
                      </div>
                    ))}
                  </>
                ) : null}
              </>
            ) : (
              <p className="text-sm text-muted-foreground">{t("apps.choose")}</p>
            )}
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
