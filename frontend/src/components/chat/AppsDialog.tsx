import { Select } from "../ui/select";
import { Input } from "../ui/input";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowLeftIcon, PlusIcon, SearchIcon } from "lucide-react";
import { Tabs } from "@base-ui/react/tabs";
import { cn } from "cn";
import { ConfirmActionDialog } from "../ui/confirm-action-dialog";
import { SettingsSection, panelTabClassName } from "../ui/control-panel";
import { Textarea } from "../ui/textarea";
import { useNavigate } from "@tanstack/react-router";
import { createConfiguredApiClient } from "../../lib/api/client";
import { useCollections } from "../../lib/electric/collections-context";
import { useActiveChannels, useUsers } from "../../hooks/useRealtime";
import { useCurrentUser } from "../../hooks/useCurrentUser";
import { useShell } from "../layout/shell-context";
import { Button } from "../ui/button";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "../ui/dialog";
import { inputClassName } from "../ui/input-styles";
import { AppCredentials, AppPermissions } from "./AppSettingsSections";
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
export function AppDialogs() {
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
export function AppsPanel({ onClose, channelId }: { onClose: () => void; channelId?: string }) {
  const { currentUserId } = useCurrentUser();
  return currentUserId ? (
    <UserAppsDialog key={currentUserId} embedded channelId={channelId} onClose={onClose} />
  ) : null;
}

function UserAppsDialog({
  embedded = false,
  channelId,
  initialId,
  messageId,
  onClose,
}: {
  embedded?: boolean;
  channelId?: string;
  initialId?: string;
  messageId?: string;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const { config } = useCollections();
  const { currentUserId, isAdmin } = useCurrentUser();
  const shell = useShell();
  const [targetChannelId, setTargetChannelId] = useState(
    channelId ?? shell.selectedChannelId ?? "",
  );
  const channels = (useActiveChannels().data ?? []).filter(
    (channel) => !channel.is_dm && !channel.archive_at,
  );

  const navigate = useNavigate();
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState("all");
  const [confirmAction, setConfirmAction] = useState<"delete" | "disable" | null>(null);
  const [detailTab, setDetailTab] = useState("overview");
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
  const visibleApps = apps
    .filter((candidate) => {
      if (
        messageId &&
        (candidate.status !== "installed" || !candidate.manifest.message_shortcuts?.length)
      )
        return false;
      if (filter === "mine" && candidate.owner_id !== currentUserId) return false;
      if (filter === "installed" && candidate.status !== "installed") return false;
      if (filter === "pending" && candidate.status !== "pending") return false;
      return `${candidate.name} ${candidate.description}`
        .toLowerCase()
        .includes(query.trim().toLowerCase());
    })
    .sort((a, b) => a.name.localeCompare(b.name));
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
    setCommandArguments({});
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
  async function confirmAppAction() {
    if (!selected || !confirmAction) return;
    await run(async () => {
      await client.request(`/api/apps/${selected}/actions/${confirmAction}`, { method: "POST" });
      if (confirmAction === "delete") {
        setSelected(undefined);
        setCredential("");
        setDeliveries([]);
        setEditing(false);
      }
      setConfirmAction(null);
    });
  }
  function edit(existing?: App) {
    setName(existing?.name ?? "");
    setDescription(existing?.description ?? "");
    setIcon(existing?.icon_url ?? "");
    setManifest(existing ? JSON.stringify(existing.manifest, null, 2) : defaultManifest);
    setDetailTab("overview");
    setEditing(true);
  }
  async function save() {
    try {
      JSON.parse(manifest);
    } catch {
      setError(t("apps.ui.invalidManifest"));
      return;
    }
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
      setDetailTab("manage");
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
          channel_id: kind === "home" ? null : targetChannelId || null,
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
      onClose();
      await navigate({ to: "/channels/$channelId", params: { channelId: result.data.id } });
    });
  }
  const content = (
    <>
      {!messageId ? (
        <label className="flex flex-wrap items-center gap-3 text-sm">
          <span>{t("apps.ui.channel")}</span>
          <Select
            className="min-w-0 w-auto max-w-full"
            value={targetChannelId}
            onChange={(event) => setTargetChannelId(event.target.value)}
          >
            <option value="">{t("apps.ui.chooseChannel")}</option>
            {channels.map((channel) => (
              <option key={channel.id} value={channel.id}>
                #{channel.channel_name}
              </option>
            ))}
          </Select>
        </label>
      ) : null}
      <ConfirmActionDialog
        open={confirmAction !== null && !!canManage}
        onOpenChange={(open) => {
          if (!open) {
            setConfirmAction(null);
            setError("");
          }
        }}
        title={
          confirmAction === "delete"
            ? t("apps.ui.deleteTitle", { name: app?.name ?? "" })
            : t("apps.disable")
        }
        description={
          confirmAction === "delete" ? t("apps.ui.deleteHelp") : t("apps.ui.disableHelp")
        }
        confirmLabel={confirmAction === "delete" ? t("apps.ui.delete") : t("apps.disable")}
        onConfirm={confirmAppAction}
        pending={busy}
        error={error}
      />
      {!embedded ? (
        <DialogHeader className="pr-8">
          <DialogTitle>{messageId ? t("apps.messageActions") : t("apps.title")}</DialogTitle>
          <DialogDescription>{t("apps.description")}</DialogDescription>
        </DialogHeader>
      ) : null}
      {error ? (
        <p
          role="alert"
          className="rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive"
        >
          {error}
        </p>
      ) : null}
      <div className="grid min-h-0 flex-1 gap-4 md:grid-cols-[14rem_minmax(0,1fr)]">
        <div
          className={cn(
            "flex min-h-0 flex-col gap-3 md:border-r md:pr-4",
            (selected || editing) && "hidden md:flex",
          )}
        >
          <label className="relative block">
            <SearchIcon
              className="pointer-events-none absolute left-2.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground"
              aria-hidden="true"
            />
            <Input
              className={`${inputClassName()} w-full pl-8`}
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder={t("apps.ui.search")}
              aria-label={t("apps.ui.search")}
            />
          </label>
          {!messageId ? (
            <Select
              className="w-auto"
              aria-label={t("apps.ui.filter")}
              value={filter}
              onChange={(event) => setFilter(event.target.value)}
            >
              <option value="all">{t("apps.ui.all")}</option>
              <option value="installed">{t("apps.status.installed")}</option>
              <option value="mine">{t("apps.ui.mine")}</option>
              {isAdmin ? <option value="pending">{t("apps.status.pending")}</option> : null}
            </Select>
          ) : null}
          <div
            className="min-h-0 max-h-[50dvh] flex-1 space-y-1 overflow-y-auto md:max-h-none"
            aria-label={t("apps.title")}
          >
            {loading ? (
              <p role="status" className="p-3 text-sm text-muted-foreground">
                {t("apps.loading")}
              </p>
            ) : null}
            {!loading && !visibleApps.length ? (
              <p className="p-3 text-sm text-muted-foreground">{t("apps.ui.empty")}</p>
            ) : null}
            {visibleApps.map((candidate) => (
              <button
                type="button"
                key={candidate.id}
                disabled={busy}
                aria-pressed={selected === candidate.id}
                className={cn(
                  "flex w-full items-center gap-3 rounded-lg p-3 text-left text-sm outline-none hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50",
                  selected === candidate.id && "bg-brand-muted text-brand-muted-foreground",
                )}
                onClick={() => {
                  setSelected(candidate.id);
                  setEditing(false);
                  setDetailTab("overview");
                  setError("");
                }}
              >
                <AppAvatar label={candidate.name} isBot src={candidate.icon_url} size="sm" />
                <span className="min-w-0 flex-1">
                  <span className="block truncate font-medium">{candidate.name}</span>
                  <span className="block text-xs text-muted-foreground">
                    {t(`apps.status.${candidate.status}`)}
                  </span>
                </span>
              </button>
            ))}
          </div>
          {error && !loading ? (
            <Button
              variant="outline"
              disabled={busy}
              onClick={() => {
                setError("");
                setRevision((v) => v + 1);
              }}
            >
              {t("offline.retry")}
            </Button>
          ) : null}
          {!messageId ? (
            <Button
              variant="outline"
              className="w-full"
              disabled={busy}
              onClick={() => {
                setSelected(undefined);
                edit();
              }}
            >
              <PlusIcon />
              {t("apps.create")}
            </Button>
          ) : null}
        </div>
        <div
          className={cn(
            "min-h-0 min-w-0 space-y-4 overflow-y-auto",
            !embedded && "md:max-h-[65dvh]",
            !selected && !editing && "hidden md:block",
          )}
        >
          {selected || editing ? (
            <Button
              variant="ghost"
              size="sm"
              className="md:hidden"
              disabled={busy}
              onClick={() => {
                setSelected(undefined);
                setEditing(false);
                setCredential("");
                setError("");
              }}
            >
              <ArrowLeftIcon />
              {t("apps.ui.back")}
            </Button>
          ) : null}
          {editing ? (
            <form
              autoComplete="off"
              className="space-y-3"
              onSubmit={(e) => {
                e.preventDefault();
                void save();
              }}
            >
              <h2 className="text-base font-semibold">
                {selected ? t("apps.edit") : t("apps.create")}
              </h2>
              <label className="flex flex-col gap-1 text-sm">
                {t("apps.name")}
                <Input
                  className={inputClassName()}
                  required
                  maxLength={80}
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                />
              </label>
              <label className="flex flex-col gap-1 text-sm">
                {t("apps.about")}
                <Textarea
                  className={inputClassName()}
                  maxLength={4000}
                  value={description}
                  onChange={(e) => setDescription(e.target.value)}
                />
              </label>
              <label className="flex flex-col gap-1 text-sm">
                {t("apps.icon")}
                <Input
                  className={inputClassName()}
                  type="url"
                  value={icon}
                  onChange={(e) => setIcon(e.target.value)}
                />
              </label>
              <label className="flex flex-col gap-1 text-sm">
                {t("apps.manifest")}
                <Textarea
                  className={`${inputClassName()} min-h-60 font-mono text-xs`}
                  value={manifest}
                  onChange={(e) => setManifest(e.target.value)}
                />
              </label>
              <div className="flex justify-end gap-2">
                <Button
                  type="button"
                  variant="outline"
                  disabled={busy}
                  onClick={() => {
                    setEditing(false);
                    setError("");
                  }}
                >
                  {t("common.cancel")}
                </Button>
                <Button type="submit" disabled={busy || !name.trim()}>
                  {t("apps.save")}
                </Button>
              </div>
            </form>
          ) : app ? (
            <>
              <div className="flex items-center gap-3">
                <AppAvatar label={app.name} isBot src={app.icon_url} />
                <div>
                  <h2 className="font-semibold">
                    {app.name} <span className="text-xs text-muted-foreground">APP</span>
                  </h2>
                  <p className="text-xs text-muted-foreground">{t(`apps.status.${app.status}`)}</p>
                </div>
              </div>
              <p className="whitespace-pre-wrap text-sm">{app.description}</p>
              <Tabs.Root value={detailTab} onValueChange={(value) => setDetailTab(String(value))}>
                <Tabs.List
                  className="mb-4 flex gap-1 rounded-lg bg-muted p-1"
                  aria-label={app.name}
                >
                  <Tabs.Tab value="overview" className={panelTabClassName}>
                    {t("apps.ui.overview")}
                  </Tabs.Tab>
                  {canManage ? (
                    <Tabs.Tab value="manage" className={panelTabClassName}>
                      {t("apps.ui.manage")}
                    </Tabs.Tab>
                  ) : null}
                </Tabs.List>
                <Tabs.Panel value="overview" className="space-y-4">
                  {app.status === "installed" ? (
                    <AppConnections
                      key={app.id}
                      appId={app.id}
                      channelId={targetChannelId || null}
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
                    targetChannelId ? (
                      <Button
                        variant="outline"
                        disabled={busy}
                        onClick={() =>
                          void run(() =>
                            client.request(`/api/apps/${app.id}/channels/${targetChannelId}`, {
                              method: "POST",
                            }),
                          )
                        }
                      >
                        {t("apps.addChannel")}
                      </Button>
                    ) : null}
                  </div>
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
                  {app.status === "installed" &&
                    app.manifest.commands?.map((command) => (
                      <div key={command} className="flex items-end gap-2">
                        <label className="flex flex-1 flex-col gap-1 text-sm">
                          /{command} {t("apps.commandArguments")}
                          <Input
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
                          disabled={busy || !targetChannelId}
                          onClick={() => void invoke("command", command)}
                        >
                          /{command}
                        </Button>
                      </div>
                    ))}
                  {app.status === "installed" &&
                    (messageId ? app.manifest.message_shortcuts : app.manifest.shortcuts)?.map(
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
                  <AppPermissions scopes={app.manifest.scopes} />
                </Tabs.Panel>
                {canManage ? (
                  <Tabs.Panel value="manage" className="space-y-4">
                    <SettingsSection title={t("apps.ui.configuration")}>
                      <div className="flex flex-wrap gap-2">
                        {canManage ? (
                          <>
                            <Button variant="outline" disabled={busy} onClick={() => edit(app)}>
                              {t("apps.edit")}
                            </Button>
                            {app.status === "draft" || app.status === "disabled" ? (
                              <Button
                                variant="outline"
                                disabled={busy}
                                onClick={() => void action("submit")}
                              >
                                {t("apps.requestApproval")}
                              </Button>
                            ) : null}
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
                            {app.status !== "disabled" ? (
                              <Button
                                variant="destructive"
                                disabled={busy}
                                onClick={() => {
                                  setError("");
                                  setConfirmAction("disable");
                                }}
                              >
                                {t("apps.disable")}
                              </Button>
                            ) : null}
                          </>
                        ) : null}
                      </div>
                    </SettingsSection>
                    <SettingsSection
                      title={t("apps.ui.delete")}
                      description={t("apps.ui.deleteHelp")}
                      className="border-destructive/30"
                    >
                      <Button
                        variant="destructive"
                        disabled={busy}
                        onClick={() => {
                          setError("");
                          setConfirmAction("delete");
                        }}
                      >
                        {t("apps.ui.delete")}
                      </Button>
                    </SettingsSection>
                    {credential ? (
                      <AppCredentials
                        value={credential}
                        onError={() => setError(t("apps.actionError"))}
                      />
                    ) : null}

                    <SettingsSection title={t("apps.connection")}>
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
                      </div>
                    </SettingsSection>
                    <AppPermissions scopes={app.manifest.scopes} />
                    <SettingsSection title={t("apps.deliveries")}>
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
                    </SettingsSection>
                  </Tabs.Panel>
                ) : null}
              </Tabs.Root>
            </>
          ) : (
            <p className="text-sm text-muted-foreground">{t("apps.choose")}</p>
          )}
        </div>
      </div>
    </>
  );
  return embedded ? (
    <div className="flex min-h-0 flex-col gap-4">{content}</div>
  ) : (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open && !busy) onClose();
      }}
    >
      <DialogContent className="flex max-h-[90dvh] flex-col overflow-hidden sm:max-w-4xl">
        {content}
      </DialogContent>
    </Dialog>
  );
}
