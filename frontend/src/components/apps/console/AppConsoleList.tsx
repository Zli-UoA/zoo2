import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowLeftIcon, PlusIcon } from "lucide-react";
import { Link, useNavigate } from "@tanstack/react-router";
import { createConfiguredApiClient } from "../../../lib/api/client";
import { useCollections } from "../../../lib/electric/collections-context";
import { describeApiError } from "../../../hooks/useChannelAdmin";
import { useCurrentUser } from "../../../hooks/useCurrentUser";
import { useShell } from "../../layout/shell-context";
import { PaneHeader } from "../../layout/PaneHeader";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";
import { Textarea } from "../../ui/textarea";
import { inputClassName } from "../../ui/input-styles";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "../../ui/dialog";
import { AppAvatar } from "../../chat/AppIdentity";
import type { AppStatus } from "./model";

type AppSummary = {
  id: string;
  name: string;
  description: string;
  icon_url: string | null;
  owner_id: string;
  status: AppStatus;
  can_manage?: boolean;
  manifest: { notification_only?: boolean };
};

/** Starting point for a new app; every field can be changed in the console before approval. */
export const NEW_APP_MANIFEST = {
  scopes: ["messages:read", "messages:write"],
  events: ["app.mentioned"],
  transport: "websocket",
};

export function AppConsoleList() {
  const { currentUserId } = useCurrentUser();
  return currentUserId ? <UserAppConsoleList key={currentUserId} /> : null;
}

function UserAppConsoleList() {
  const { t } = useTranslation();
  const { config } = useCollections();
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  const { currentUserId, isAdmin } = useCurrentUser();
  const shell = useShell();
  const [apps, setApps] = useState<AppSummary[] | null>(null);
  const [failed, setFailed] = useState(false);
  const [reload, setReload] = useState(0);
  const [creating, setCreating] = useState(false);
  useEffect(() => {
    const controller = new AbortController();
    setFailed(false);
    client
      .get<AppSummary[]>("/api/apps", { signal: controller.signal })
      .then((next) => {
        if (!controller.signal.aborted) setApps(next);
      })
      .catch(() => {
        if (!controller.signal.aborted) setFailed(true);
      });
    return () => controller.abort();
  }, [client, reload]);
  const managed = (app: AppSummary) =>
    app.can_manage ?? (app.owner_id === currentUserId || isAdmin);
  const groups: { key: string; title: string; apps: AppSummary[] }[] = apps
    ? [
        ...(isAdmin
          ? [
              {
                key: "pending",
                title: t("appConsole.list.pending"),
                apps: apps.filter((app) => app.status === "pending"),
              },
            ]
          : []),
        {
          key: "mine",
          title: t("appConsole.list.mine"),
          apps: apps.filter((app) => app.owner_id === currentUserId),
        },
        {
          key: "installed",
          title: t("appConsole.list.installed"),
          apps: apps.filter((app) => app.status === "installed" && app.owner_id !== currentUserId),
        },
      ]
    : [];
  return (
    <section className="flex h-full min-h-0 flex-col" aria-labelledby="app-console-list-title">
      <PaneHeader>
        <Button
          variant="ghost"
          size="icon-sm"
          className="size-11 md:hidden"
          aria-label={t("channels.back")}
          onClick={shell.showSidebar}
        >
          <ArrowLeftIcon />
        </Button>
        <h1 id="app-console-list-title" className="min-w-0 flex-1 truncate text-lg font-semibold">
          {t("appConsole.list.title")}
        </h1>
        <Button size="sm" onClick={() => setCreating(true)}>
          <PlusIcon />
          {t("apps.create")}
        </Button>
      </PaneHeader>
      <div className="min-h-0 flex-1 space-y-6 overflow-y-auto p-4 sm:p-6">
        <p className="text-sm text-muted-foreground">{t("appConsole.list.description")}</p>
        {failed ? (
          <div role="alert" className="flex flex-wrap items-center gap-2 text-sm">
            <span className="text-destructive">{t("apps.loadError")}</span>
            <Button variant="outline" size="sm" onClick={() => setReload((value) => value + 1)}>
              {t("offline.retry")}
            </Button>
          </div>
        ) : !apps ? (
          <p role="status" className="text-sm text-muted-foreground">
            {t("apps.loading")}
          </p>
        ) : null}
        {groups.map((group) => (
          <section key={group.key} aria-labelledby={`apps-${group.key}`} className="space-y-2">
            <h2 id={`apps-${group.key}`} className="text-sm font-semibold">
              {group.title}
            </h2>
            {group.apps.length === 0 ? (
              <p className="text-sm text-muted-foreground">{t("apps.ui.empty")}</p>
            ) : (
              <ul className="divide-y rounded-lg border">
                {group.apps.map((app) => (
                  <li key={app.id}>
                    <Link
                      to="/apps/$appId"
                      params={{ appId: app.id }}
                      search={{ section: "basic" }}
                      className="flex items-center gap-3 p-3 text-sm outline-none hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring"
                    >
                      <AppAvatar label={app.name} isBot src={app.icon_url} size="sm" />
                      <span className="min-w-0 flex-1">
                        <span className="block truncate font-medium">{app.name}</span>
                        <span className="block truncate text-xs text-muted-foreground">
                          {t(`apps.status.${app.status}`)}
                          {app.manifest?.notification_only
                            ? ` · ${t("appConsole.list.notificationOnly")}`
                            : ""}
                          {managed(app) ? "" : ` · ${t("appConsole.list.viewOnly")}`}
                        </span>
                      </span>
                    </Link>
                  </li>
                ))}
              </ul>
            )}
          </section>
        ))}
      </div>
      {creating ? <CreateAppDialog onClose={() => setCreating(false)} /> : null}
    </section>
  );
}

function CreateAppDialog({ onClose }: { onClose: () => void }) {
  const { t } = useTranslation();
  const { config } = useCollections();
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  const navigate = useNavigate();
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open && !busy) onClose();
      }}
    >
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>{t("apps.create")}</DialogTitle>
          <DialogDescription>{t("appConsole.list.createHelp")}</DialogDescription>
        </DialogHeader>
        <form
          autoComplete="off"
          className="space-y-3"
          onSubmit={(event) => {
            event.preventDefault();
            if (!name.trim() || busy) return;
            setBusy(true);
            setError("");
            client
              .request<{ id: string }>("/api/apps", {
                method: "POST",
                body: { name: name.trim(), description, manifest: NEW_APP_MANIFEST },
              })
              .then((created) =>
                navigate({
                  to: "/apps/$appId",
                  params: { appId: created.id },
                  search: { section: "basic" },
                }),
              )
              .catch((cause) => {
                setError(describeApiError(cause, t));
                setBusy(false);
              });
          }}
        >
          <label className="flex flex-col gap-1 text-sm">
            {t("apps.name")}
            <Input
              className={inputClassName()}
              required
              maxLength={80}
              value={name}
              onChange={(event) => setName(event.target.value)}
            />
          </label>
          <label className="flex flex-col gap-1 text-sm">
            {t("apps.about")}
            <Textarea
              className={inputClassName()}
              maxLength={4000}
              value={description}
              onChange={(event) => setDescription(event.target.value)}
            />
          </label>
          {error ? (
            <p role="alert" className="text-sm text-destructive">
              {error}
            </p>
          ) : null}
          <div className="flex justify-end gap-2">
            <Button type="button" variant="outline" disabled={busy} onClick={onClose}>
              {t("common.cancel")}
            </Button>
            <Button type="submit" disabled={busy || !name.trim()}>
              {t("apps.create")}
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}
