import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowLeftIcon } from "lucide-react";
import { useNavigate } from "@tanstack/react-router";
import { cn } from "cn";
import { ApiError, createConfiguredApiClient, type ApiClient } from "../../../lib/api/client";
import { useCollections } from "../../../lib/electric/collections-context";
import { describeApiError } from "../../../hooks/useChannelAdmin";
import { useCurrentUser } from "../../../hooks/useCurrentUser";
import { useUsers } from "../../../hooks/useRealtime";
import { Button } from "../../ui/button";
import { Select } from "../../ui/select";
import { PaneHeader } from "../../layout/PaneHeader";
import { AppAvatar } from "../../chat/AppIdentity";
import { AppPermissions } from "../../chat/AppSettingsSections";
import {
  type AppDetail,
  type AppDraft,
  type ConsoleSection,
  type EventCatalog,
  dirtySections,
  draftFromDetail,
  isDirty,
  needsReapproval,
  saveBody,
  visibleSections,
} from "./model";
import {
  BasicSection,
  EventsSection,
  HomeSection,
  InteractivitySection,
  ManifestSection,
  PermissionsSection,
} from "./ConfigSections";
import {
  CredentialsSection,
  DeliveriesSection,
  HistorySection,
  InstallSection,
  WebhooksSection,
} from "./OperationSections";

/** Everything a section needs: the shared draft, the loaded detail and a way to run server actions. */
export type SectionProps = {
  detail: AppDetail;
  saved: AppDraft;
  draft: AppDraft;
  setDraft: (update: (draft: AppDraft) => AppDraft) => void;
  catalog: EventCatalog | null;
  client: ApiClient;
  isAdmin: boolean;
  /** Reloads the detail after a server action, keeping unsaved edits. */
  refresh: () => void;
};

export function AppConsole({ appId, section }: { appId: string; section: ConsoleSection }) {
  const { currentUserId } = useCurrentUser();
  return currentUserId ? (
    <UserAppConsole key={`${currentUserId}:${appId}`} appId={appId} section={section} />
  ) : null;
}

function UserAppConsole({ appId, section }: { appId: string; section: ConsoleSection }) {
  const { t, i18n } = useTranslation();
  const { config } = useCollections();
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  const navigate = useNavigate();
  const { currentUserId, isAdmin } = useCurrentUser();
  const people = useUsers().data ?? [];
  const [detail, setDetail] = useState<AppDetail | null>(null);
  const [saved, setSaved] = useState<AppDraft | null>(null);
  const [draft, setDraftState] = useState<AppDraft | null>(null);
  const [catalog, setCatalog] = useState<EventCatalog | null>(null);
  const [loadError, setLoadError] = useState(false);
  // `keep` travels with each reload request, so a failed or aborted refresh cannot leak into the next load.
  const [reload, setReload] = useState({ count: 0, keep: false });
  const reloadDetail = (keep: boolean) => setReload((value) => ({ count: value.count + 1, keep }));
  const [saving, setSaving] = useState(false);
  const savingNow = useRef(false);
  const [saveError, setSaveError] = useState("");
  const [conflict, setConflict] = useState(false);
  const botUsername = people.find((user) => user.id === detail?.bot_user_id)?.user_name ?? "";
  const latestBotUsername = useRef(botUsername);
  latestBotUsername.current = botUsername;
  // The conflict check compares against the version the edits started from, not the latest fetch.
  const [expectedUpdatedAt, setExpectedUpdatedAt] = useState("");
  const latestSaved = useRef(saved);
  latestSaved.current = saved;
  const latestDraft = useRef(draft);
  latestDraft.current = draft;

  useEffect(() => {
    const controller = new AbortController();
    setLoadError(false);
    client
      .get<AppDetail>(`/api/apps/${encodeURIComponent(appId)}`, { signal: controller.signal })
      .then((next) => {
        if (controller.signal.aborted) return;
        const fresh = draftFromDetail(next, latestBotUsername.current);
        const base = latestSaved.current;
        // Server actions (approval, tokens) refresh the detail without discarding edits in progress.
        const editing =
          reload.keep && base && latestDraft.current && isDirty(base, latestDraft.current);
        setDetail(next);
        // During a save this load may already see the user's own changes; the reload after the
        // save sets the new baseline, so leave it alone here.
        if (editing && savingNow.current) return;
        if (editing && !sameSettings(base, fresh)) {
          // Someone else saved while these edits were open. Keep the old baseline and
          // `updated_at` so the save is still rejected, and report the conflict now.
          setConflict(true);
        } else {
          setSaved(fresh);
          setExpectedUpdatedAt(next.updated_at);
          if (!editing) {
            setDraftState(fresh);
            setConflict(false);
          }
        }
      })
      .catch(() => {
        if (!controller.signal.aborted) setLoadError(true);
      });
    return () => controller.abort();
    // The baseline is read through refs so edits do not trigger a reload.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [client, appId, reload]);

  useEffect(() => {
    const controller = new AbortController();
    client
      .get<EventCatalog>("/api/apps/events", { signal: controller.signal })
      .then((next) => {
        if (!controller.signal.aborted) setCatalog(next);
      })
      .catch(() => {});
    return () => controller.abort();
  }, [client]);

  // The bot username comes from the user list, which can arrive after the app detail.
  useEffect(() => {
    if (!botUsername) return;
    setSaved((current) =>
      current && !current.bot_username ? { ...current, bot_username: botUsername } : current,
    );
    setDraftState((current) =>
      current && !current.bot_username ? { ...current, bot_username: botUsername } : current,
    );
  }, [botUsername]);

  function selectSection(next: ConsoleSection) {
    void navigate({
      to: "/apps/$appId",
      params: { appId },
      search: { section: next },
      replace: true,
    });
  }
  const back = (
    <Button
      variant="ghost"
      size="icon-sm"
      className="size-11"
      aria-label={t("appConsole.back")}
      onClick={() => void navigate({ to: "/apps" })}
    >
      <ArrowLeftIcon />
    </Button>
  );

  if (!detail || !saved || !draft) {
    return (
      <section className="flex h-full min-h-0 flex-col" aria-labelledby="app-console-title">
        <PaneHeader>
          {back}
          <h1 id="app-console-title" className="text-lg font-semibold">
            {t("appConsole.title")}
          </h1>
        </PaneHeader>
        <div className="p-6">
          {loadError ? (
            <div role="alert" className="space-y-3 text-sm">
              <p className="text-destructive">{t("appConsole.loadError")}</p>
              <Button variant="outline" onClick={() => reloadDetail(false)}>
                {t("offline.retry")}
              </Button>
            </div>
          ) : (
            <p role="status" className="text-sm text-muted-foreground">
              {t("apps.loading")}
            </p>
          )}
        </div>
      </section>
    );
  }

  const canManage = detail.owner_id === currentUserId || isAdmin;
  const sections = visibleSections(draft.manifest);
  const current = sections.includes(section) ? section : "basic";
  const dirty = isDirty(saved, draft);
  const changed = dirtySections(saved, draft);
  const props: SectionProps = {
    detail,
    saved,
    draft,
    setDraft: (update) => setDraftState((value) => (value ? update(value) : value)),
    catalog,
    client,
    isAdmin,
    refresh: () => {
      reloadDetail(true);
    },
  };

  async function save() {
    if (!detail || !saved || !draft || saving) return;
    setSaving(true);
    savingNow.current = true;
    setSaveError("");
    try {
      await client.request(`/api/apps/${encodeURIComponent(appId)}`, {
        method: "PATCH",
        body: saveBody(draft, saved, expectedUpdatedAt),
      });
      reloadDetail(false);
    } catch (cause) {
      if (cause instanceof ApiError && cause.status === 409) setConflict(true);
      else setSaveError(describeApiError(cause, t));
    } finally {
      setSaving(false);
      savingNow.current = false;
    }
  }

  return (
    <section className="flex h-full min-h-0 flex-col" aria-labelledby="app-console-title">
      <PaneHeader>
        {back}
        <AppAvatar label={detail.name} isBot src={detail.icon_url} size="sm" />
        <div className="min-w-0">
          <h1 id="app-console-title" className="truncate text-lg font-semibold">
            {detail.name}
          </h1>
          <p className="text-xs text-muted-foreground">
            {t(`apps.status.${detail.status}`)} ·{" "}
            {t("appConsole.revision", { revision: detail.revision })}
          </p>
        </div>
      </PaneHeader>
      {!canManage ? (
        <div className="min-h-0 flex-1 space-y-4 overflow-y-auto p-4 sm:p-6">
          <p className="text-sm text-muted-foreground">{t("appConsole.readOnly")}</p>
          <p className="whitespace-pre-wrap text-sm">{detail.description}</p>
          <AppPermissions scopes={draft.manifest.scopes} />
        </div>
      ) : (
        <div className="grid min-h-0 flex-1 md:grid-cols-[13rem_minmax(0,1fr)]">
          <nav
            aria-label={t("appConsole.sections")}
            className="hidden min-h-0 overflow-y-auto border-r p-3 md:block"
          >
            <ul className="space-y-0.5">
              {sections.map((item) => (
                <li key={item}>
                  <button
                    type="button"
                    aria-current={item === current ? "page" : undefined}
                    className={cn(
                      "flex w-full items-center justify-between gap-2 rounded-md px-3 py-2 text-left text-sm outline-none hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring",
                      item === current && "bg-brand-muted font-medium text-brand-muted-foreground",
                    )}
                    onClick={() => selectSection(item)}
                  >
                    {t(`appConsole.section.${item}`)}
                    {changed.includes(item) ? (
                      <span
                        className="size-2 rounded-full bg-primary"
                        aria-label={t("appConsole.unsaved")}
                      />
                    ) : null}
                  </button>
                </li>
              ))}
            </ul>
          </nav>
          <div className="flex min-h-0 flex-col">
            <div className="min-h-0 flex-1 space-y-4 overflow-y-auto p-4 sm:p-6">
              <label className="flex flex-col gap-1 text-sm md:hidden">
                {t("appConsole.sections")}
                <Select
                  value={current}
                  onChange={(event) => selectSection(event.target.value as ConsoleSection)}
                >
                  {sections.map((item) => (
                    <option key={item} value={item}>
                      {t(`appConsole.section.${item}`)}
                    </option>
                  ))}
                </Select>
              </label>
              <h2 className="text-base font-semibold">{t(`appConsole.section.${current}`)}</h2>
              <SectionBody section={current} {...props} />
            </div>
            {dirty || conflict ? (
              <div
                role="region"
                aria-label={t("appConsole.unsaved")}
                className="space-y-2 border-t bg-background p-3 sm:px-6"
              >
                {conflict ? (
                  <div role="alert" className="flex flex-wrap items-center gap-2 text-sm">
                    <span className="text-destructive">{t("appConsole.conflict")}</span>
                    <Button
                      variant="outline"
                      size="sm"
                      onClick={() => {
                        setConflict(false);
                        reloadDetail(false);
                      }}
                    >
                      {t("appConsole.reload")}
                    </Button>
                  </div>
                ) : null}
                {saveError ? (
                  <p role="alert" className="text-sm text-destructive">
                    {saveError}
                  </p>
                ) : null}
                {dirty ? (
                  <div className="flex flex-wrap items-center gap-2">
                    <p className="min-w-0 flex-1 text-xs text-muted-foreground">
                      {t("appConsole.unsavedSections", {
                        sections: new Intl.ListFormat(i18n.language).format(
                          changed.map((item) => t(`appConsole.section.${item}`)),
                        ),
                      })}
                      {needsReapproval(saved, draft) && detail.status !== "draft"
                        ? ` ${t("appConsole.reapproval")}`
                        : ""}
                    </p>
                    <Button
                      variant="outline"
                      disabled={saving}
                      onClick={() => {
                        setDraftState(saved);
                        setSaveError("");
                      }}
                    >
                      {t("appConsole.discard")}
                    </Button>
                    <Button
                      disabled={saving || conflict || !draft.name.trim()}
                      onClick={() => void save()}
                    >
                      {t("apps.save")}
                    </Button>
                  </div>
                ) : null}
              </div>
            ) : null}
          </div>
        </div>
      )}
    </section>
  );
}

function SectionBody({ section, ...props }: SectionProps & { section: ConsoleSection }) {
  switch (section) {
    case "basic":
      return <BasicSection {...props} />;
    case "credentials":
      return <CredentialsSection {...props} />;
    case "permissions":
      return <PermissionsSection {...props} />;
    case "events":
      return <EventsSection {...props} />;
    case "webhooks":
      return <WebhooksSection {...props} />;
    case "interactivity":
      return <InteractivitySection {...props} />;
    case "home":
      return <HomeSection {...props} />;
    case "install":
      return <InstallSection {...props} />;
    case "deliveries":
      return <DeliveriesSection {...props} />;
    case "history":
      return <HistorySection {...props} />;
    case "manifest":
      return <ManifestSection {...props} />;
  }
}

/**
 * Whether two snapshots hold the same settings. The bot username comes from the user list,
 * which can load later, so it is compared only when both snapshots know it.
 */
function sameSettings(a: AppDraft, b: AppDraft) {
  const known = Boolean(a.bot_username && b.bot_username);
  return !isDirty(known ? a : { ...a, bot_username: "" }, known ? b : { ...b, bot_username: "" });
}
