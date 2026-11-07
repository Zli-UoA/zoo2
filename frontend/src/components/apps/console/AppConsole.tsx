import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowLeftIcon } from "lucide-react";
import { useNavigate } from "@tanstack/react-router";
import { cn } from "cn";
import { ApiError, createConfiguredApiClient, type ApiClient } from "../../../lib/api/client";
import { useCollections } from "../../../lib/electric/collections-context";
import { describeApiError } from "../../../hooks/useChannelAdmin";
import { useCurrentUser } from "../../../hooks/useCurrentUser";
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
  const [detail, setDetail] = useState<AppDetail | null>(null);
  const [saved, setSaved] = useState<AppDraft | null>(null);
  const [draft, setDraftState] = useState<AppDraft | null>(null);
  const [catalog, setCatalog] = useState<EventCatalog | null>(null);
  const [loadError, setLoadError] = useState(false);
  // Each reload carries its own purpose, so a failed or aborted reload cannot leak into the next one.
  // `keep`: an action refreshed the detail and edits in progress stay.
  // `sent`: the reload after this screen's own save, with the draft that was saved.
  const [reload, setReload] = useState<{ count: number; keep: boolean; sent?: AppDraft }>({
    count: 0,
    keep: false,
  });
  const reloadDetail = (keep: boolean, sent?: AppDraft) =>
    setReload((value) => ({ count: value.count + 1, keep, sent }));
  const latestReloadCount = useRef(reload.count);
  latestReloadCount.current = reload.count;
  // Loads requested before this screen's last save started or finished may hold either version;
  // the reload after the save decides, so earlier ones are ignored.
  const staleBefore = useRef(0);
  // `updated_at` returned by this screen's last save.
  const savedVersion = useRef("");
  const [saving, setSaving] = useState(false);
  const savingNow = useRef(false);
  const [saveError, setSaveError] = useState("");
  const [conflict, setConflict] = useState(false);
  // The conflict check compares against the version the edits started from, not the latest fetch.
  const [expectedUpdatedAt, setExpectedUpdatedAt] = useState("");
  const latestExpected = useRef(expectedUpdatedAt);
  latestExpected.current = expectedUpdatedAt;
  const latestSaved = useRef(saved);
  latestSaved.current = saved;
  const latestDraft = useRef(draft);
  latestDraft.current = draft;

  useEffect(() => {
    const controller = new AbortController();
    const { count, keep, sent } = reload;
    setLoadError(false);
    client
      .get<AppDetail>(`/api/apps/${encodeURIComponent(appId)}`, { signal: controller.signal })
      .then((next) => {
        if (controller.signal.aborted || count < staleBefore.current) return;
        const fresh = draftFromDetail(next);
        const base = latestSaved.current;
        setDetail(next);
        if (sent) {
          if (savedVersion.current && next.updated_at !== savedVersion.current) {
            // Another save landed between this save and its reload. Keep `updated_at` from
            // this save so the next save is rejected, and report the conflict now.
            setConflict(true);
            return;
          }
          // The saved version becomes the baseline. Anything typed while the save was in
          // flight differs from the sent draft and stays.
          setSaved(fresh);
          setExpectedUpdatedAt(next.updated_at);
          setConflict(false);
          setDraftState((current) => (current && isDirty(sent, current) ? current : fresh));
          return;
        }
        // Server actions (approval, tokens) refresh the detail without discarding edits in progress.
        const editing = keep && base && latestDraft.current && isDirty(base, latestDraft.current);
        // During a save this load may already see the user's own changes; the reload after the
        // save sets the new baseline, so leave it alone here.
        if (editing && savingNow.current) return;
        // The same version as the baseline means nobody else saved, even if the server
        // normalized a value (for example trimmed whitespace) differently from the draft.
        const sameVersion = next.updated_at === latestExpected.current;
        if (editing && !sameVersion && isDirty(base, fresh)) {
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
    // Loads already requested (including a previous save's reload) predate this save and could
    // report it as a conflict or clear a conflict it finds, so they are ignored.
    staleBefore.current = latestReloadCount.current + 1;
    setSaveError("");
    try {
      const body = saveBody(draft, saved, expectedUpdatedAt);
      const result = await client.request<{ updated_at?: string }>(
        `/api/apps/${encodeURIComponent(appId)}`,
        { method: "PATCH", body },
      );
      // Until the reload below lands, an action's refresh compares against the saved version.
      savedVersion.current = result?.updated_at ?? "";
      if (result?.updated_at) setExpectedUpdatedAt(result.updated_at);
      // What was just saved is the baseline for the next save even before the reload lands, so
      // changing a value back (for example the bot username) is still sent.
      setSaved(draft);
      staleBefore.current = latestReloadCount.current + 1;
      reloadDetail(false, draft);
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
