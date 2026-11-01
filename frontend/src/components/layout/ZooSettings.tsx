import { useCallback, useEffect, useMemo, useState } from "react";
import { createConfiguredApiClient } from "../../lib/api/client";
import { useCollections } from "../../lib/electric/collections-context";
import { Textarea } from "../ui/textarea";
import { Input } from "../ui/input";
import { SettingsSection } from "../ui/control-panel";
import { ArrowLeftIcon, BellIcon, LanguagesIcon, PaletteIcon, SearchIcon } from "lucide-react";
import { Tabs } from "@base-ui/react/tabs";
import { useNavigate, useRouterState } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import { SearchEngineSelect } from "../preferences/SearchEngineSelect";
import { LanguageToggle } from "../i18n/LanguageToggle";
import { ModeToggle } from "../theme/ModeToggle";
import { PwaControls } from "../pwa/PwaControls";
import { AppsPanel } from "../chat/AppsDialog";
import { WorkspaceMembers } from "./WorkspaceMembers";
import { Button } from "../ui/button";
import { SettingRow, panelTabClassName } from "../ui/control-panel";
import { useShell } from "./shell-context";
import { PaneHeader } from "./PaneHeader";

export function ZooSettings() {
  const { t } = useTranslation();
  const shell = useShell();
  const navigate = useNavigate();
  const search = useRouterState({
    select: (state) => state.location.search as { tab?: string; channel?: string },
  });
  const tab =
    search.tab === "apps" || search.tab === "members" || search.tab === "activitypub"
      ? search.tab
      : "general";
  return (
    <section className="flex h-full min-h-0 flex-col" aria-labelledby="zoo-settings-title">
      <PaneHeader>
        <Button
          variant="ghost"
          size="icon-sm"
          className="size-11"
          aria-label={t("channels.back")}
          onClick={shell.showSidebar}
        >
          <ArrowLeftIcon />
        </Button>
        <h1 id="zoo-settings-title" className="min-w-0 text-lg font-semibold">
          {t("settings.zooTitle")}
        </h1>
      </PaneHeader>
      <div className="min-h-0 flex-1 overflow-y-auto p-4 sm:p-6">
        <Tabs.Root
          value={tab}
          onValueChange={(value) =>
            void navigate({
              to: "/settings",
              search: { ...search, tab: value as "general" | "apps" | "members" | "activitypub" },
              replace: true,
            })
          }
          className="flex w-full min-h-0 flex-col gap-4"
        >
          <Tabs.List
            className="flex shrink-0 flex-wrap gap-1 rounded-lg bg-muted p-1"
            aria-label={t("settings.zooTitle")}
          >
            <Tabs.Tab value="general" className={panelTabClassName}>
              {t("settings.general")}
            </Tabs.Tab>
            <Tabs.Tab value="activitypub" className={panelTabClassName}>
              ActivityPub
            </Tabs.Tab>
            <Tabs.Tab value="apps" className={panelTabClassName}>
              {t("apps.title")}
            </Tabs.Tab>
            <Tabs.Tab value="members" className={panelTabClassName}>
              {t("members.title")}
            </Tabs.Tab>
          </Tabs.List>
          <Tabs.Panel value="general" keepMounted className="min-h-0 data-[hidden]:hidden">
            <h2 className="sr-only">{t("settings.general")}</h2>
            <div className="divide-y rounded-lg border">
              <SettingRow icon={<PaletteIcon />} label={t("theme.title")}>
                <ModeToggle />
              </SettingRow>
              <SettingRow icon={<LanguagesIcon />} label={t("language.title")}>
                <LanguageToggle presentation="select" />
              </SettingRow>
              <SettingRow icon={<SearchIcon />} label={t("search.engine")}>
                <SearchEngineSelect />
              </SettingRow>
              <SettingRow icon={<BellIcon />} label={t("pwa.title")}>
                <PwaControls presentation="select" />
              </SettingRow>
            </div>
          </Tabs.Panel>
          <Tabs.Panel value="activitypub" className="min-h-0">
            <h2 className="sr-only">ActivityPub</h2>
            <ActivityPubSettings />
            <ChannelTimelineSettings />
          </Tabs.Panel>
          <Tabs.Panel value="apps" className="min-h-0">
            <AppsPanel channelId={search.channel} onClose={() => {}} />
          </Tabs.Panel>
          <Tabs.Panel value="members" className="min-h-0">
            <WorkspaceMembers />
          </Tabs.Panel>
        </Tabs.Root>
      </div>
    </section>
  );
}

/** Workspace-level channel publication rules belong in settings, outside the timeline. */
function ChannelTimelineSettings() {
  const { t } = useTranslation();
  const { config } = useCollections();
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  const [settings, setSettings] = useState<{
    can_manage: boolean;
    ltl_channel_pattern: string;
  } | null>(null);
  const [pattern, setPattern] = useState("");
  const [failed, setFailed] = useState(false);
  const [pending, setPending] = useState(false);
  const load = useCallback(async () => {
    setFailed(false);
    try {
      const value = await client.get<NonNullable<typeof settings>>("/api/home/settings");
      setSettings(value);
      setPattern(value.ltl_channel_pattern);
    } catch {
      setFailed(true);
    }
  }, [client]);
  useEffect(() => {
    void load();
  }, [load]);
  if (!failed && !settings?.can_manage) return null;
  return (
    <SettingsSection
      className="mt-4"
      title={t("home.channelRule")}
      description={t("home.ruleHint")}
    >
      {settings?.can_manage ? (
        <form
          className="space-y-2"
          onSubmit={(event) => {
            event.preventDefault();
            if (pending) return;
            setPending(true);
            setFailed(false);
            void client
              .request("/api/home/settings", {
                method: "PUT",
                body: { ltl_channel_pattern: pattern },
              })
              .catch(() => setFailed(true))
              .finally(() => setPending(false));
          }}
        >
          <Input
            aria-label={t("home.channelRule")}
            value={pattern}
            onChange={(event) => setPattern(event.target.value)}
            placeholder="^public-.*$"
            maxLength={200}
            disabled={pending}
          />
          <Button type="submit" variant="outline" disabled={pending}>
            {t("home.save")}
          </Button>
        </form>
      ) : null}
      {failed ? (
        <p role="alert" className="text-xs text-destructive">
          {t("home.controlsError")}{" "}
          <Button variant="link" onClick={() => void load()}>
            {t("home.refresh")}
          </Button>
        </p>
      ) : null}
    </SettingsSection>
  );
}

function ActivityPubSettings() {
  const { t } = useTranslation();
  const { config } = useCollections();
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  const [profile, setProfile] = useState<{
    display_name: string;
    default_display_name: string;
    summary: string;
  } | null>(null);
  const [name, setName] = useState("");
  const [summary, setSummary] = useState("");
  const [failed, setFailed] = useState(false);
  const [pending, setPending] = useState(false);
  const [saved, setSaved] = useState(false);
  const load = useCallback(
    async (signal?: AbortSignal) => {
      setFailed(false);
      try {
        const value = await client.get<NonNullable<typeof profile>>("/api/activitypub/profile", {
          signal,
        });
        if (signal?.aborted) return;
        setProfile(value);
        setName(value.display_name);
        setSummary(value.summary);
      } catch {
        if (!signal?.aborted) setFailed(true);
      }
    },
    [client],
  );
  useEffect(() => {
    const controller = new AbortController();
    void load(controller.signal);
    return () => controller.abort();
  }, [load]);
  return (
    <SettingsSection title="ActivityPub" description={t("activitypubSettings.description")}>
      {profile ? (
        <form
          className="space-y-3"
          onSubmit={(event) => {
            event.preventDefault();
            if (pending) return;
            setPending(true);
            setFailed(false);
            setSaved(false);
            void client
              .request<NonNullable<typeof profile>>("/api/activitypub/profile", {
                method: "PUT",
                body: { display_name: name, summary },
              })
              .then((value) => {
                setProfile(value);
                setName(value.display_name);
                setSummary(value.summary);
                setSaved(true);
              })
              .catch(() => setFailed(true))
              .finally(() => setPending(false));
          }}
        >
          <label className="block space-y-2 text-sm">
            <span className="block">{t("activitypubSettings.displayName")}</span>
            <Input
              className="w-full"
              value={name}
              onChange={(event) => {
                setName(event.target.value);
                setSaved(false);
              }}
              placeholder={profile.default_display_name}
              disabled={pending}
              aria-describedby="apub-display-name-hint"
            />
          </label>
          <p id="apub-display-name-hint" className="text-sm text-muted-foreground">
            {t("activitypubSettings.nameHint")}
          </p>
          <label className="block space-y-2 text-sm">
            <span className="block">{t("activitypubSettings.bio")}</span>
            <Textarea
              value={summary}
              onChange={(event) => {
                setSummary(event.target.value);
                setSaved(false);
              }}
              disabled={pending}
              rows={4}
            />
          </label>
          <Button
            type="submit"
            disabled={
              pending || Array.from(name.trim()).length > 100 || Array.from(summary).length > 1000
            }
          >
            {t("home.save")}
          </Button>
          {saved ? (
            <p role="status" className="text-sm">
              {t("activitypubSettings.saved")}
            </p>
          ) : null}
        </form>
      ) : !failed ? (
        <p role="status">{t("activitypubSettings.loading")}</p>
      ) : null}
      {failed ? (
        <p role="alert" className="text-sm text-destructive">
          {t("home.controlsError")}{" "}
          {!profile ? (
            <Button variant="link" onClick={() => void load()}>
              {t("home.refresh")}
            </Button>
          ) : null}
        </p>
      ) : null}
    </SettingsSection>
  );
}
