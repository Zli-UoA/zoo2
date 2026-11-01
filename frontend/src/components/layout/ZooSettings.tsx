import { useCallback, useEffect, useMemo, useState } from "react";
import { createConfiguredApiClient } from "../../lib/api/client";
import { useCollections } from "../../lib/electric/collections-context";
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
import { SlackImportPanel } from "./SlackImportPanel";
import { SlackLinkPanel } from "./SlackLinkPanel";
import { useCurrentUser } from "../../hooks/useCurrentUser";
import type { SettingsTab } from "../../routes/_shell.settings";
import { Button } from "../ui/button";
import { SettingRow, panelTabClassName } from "../ui/control-panel";
import { useShell } from "./shell-context";
import { PaneHeader } from "./PaneHeader";

const ADMIN_TABS: readonly SettingsTab[] = ["slack-import"];

export function ZooSettings() {
  const { t } = useTranslation();
  const shell = useShell();
  const navigate = useNavigate();
  const search = useRouterState({
    select: (state) => state.location.search as { tab?: SettingsTab; channel?: string },
  });
  const { isAdmin } = useCurrentUser();
  // 管理者向けのタブは、権限がなければ一般タブに戻す
  const tab: SettingsTab =
    search.tab && (isAdmin || !ADMIN_TABS.includes(search.tab)) ? search.tab : "general";
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
              search: { ...search, tab: value as SettingsTab },
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
            <Tabs.Tab value="apps" className={panelTabClassName}>
              {t("apps.title")}
            </Tabs.Tab>
            <Tabs.Tab value="members" className={panelTabClassName}>
              {t("members.title")}
            </Tabs.Tab>
            <Tabs.Tab value="slack" className={panelTabClassName}>
              {t("slackLink.tab")}
            </Tabs.Tab>
            {isAdmin ? (
              <Tabs.Tab value="slack-import" className={panelTabClassName}>
                {t("slackImport.tab")}
              </Tabs.Tab>
            ) : null}
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
            <ChannelTimelineSettings />
          </Tabs.Panel>
          <Tabs.Panel value="apps" className="min-h-0">
            <AppsPanel channelId={search.channel} onClose={() => {}} />
          </Tabs.Panel>
          <Tabs.Panel value="members" className="min-h-0">
            <WorkspaceMembers />
          </Tabs.Panel>
          <Tabs.Panel value="slack" className="min-h-0">
            <SlackLinkPanel />
          </Tabs.Panel>
          {isAdmin ? (
            <Tabs.Panel value="slack-import" className="min-h-0">
              <SlackImportPanel />
            </Tabs.Panel>
          ) : null}
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
            placeholder="public-%"
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
