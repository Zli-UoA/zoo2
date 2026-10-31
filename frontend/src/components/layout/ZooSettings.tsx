import { ArrowLeftIcon } from "lucide-react";
import { Tabs } from "@base-ui/react/tabs";
import { useNavigate, useRouterState } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import { LanguageToggle } from "../i18n/LanguageToggle";
import { ModeToggle } from "../theme/ModeToggle";
import { PwaControls } from "../pwa/PwaControls";
import { AppsPanel } from "../chat/AppsDialog";
import { WorkspaceMembers } from "./WorkspaceMembers";
import { Button } from "../ui/button";
import { SettingRow, panelTabClassName } from "../ui/control-panel";
import { useShell } from "./shell-context";

export function ZooSettings() {
  const { t } = useTranslation();
  const shell = useShell();
  const navigate = useNavigate();
  const search = useRouterState({
    select: (state) => state.location.search as { tab?: string; channel?: string },
  });
  const tab = search.tab === "apps" || search.tab === "members" ? search.tab : "general";
  return (
    <section className="flex h-full min-h-0 flex-col" aria-labelledby="zoo-settings-title">
      <header className="flex min-h-16 shrink-0 items-center gap-3 border-b px-4 py-3">
        <Button
          variant="ghost"
          size="icon-sm"
          className="size-11"
          aria-label={t("channels.back")}
          onClick={shell.showSidebar}
        >
          <ArrowLeftIcon />
        </Button>
        <div className="min-w-0">
          <h1 id="zoo-settings-title" className="text-lg font-semibold">
            {t("settings.zooTitle")}
          </h1>
          <p className="text-xs text-muted-foreground">{t("settings.zooDescription")}</p>
        </div>
      </header>
      <div className="min-h-0 flex-1 overflow-y-auto p-4 sm:p-6">
        <Tabs.Root
          value={tab}
          onValueChange={(value) =>
            void navigate({
              to: "/settings",
              search: { ...search, tab: value as "general" | "apps" | "members" },
              replace: true,
            })
          }
          className="mx-auto flex w-full max-w-5xl min-h-0 flex-col gap-4"
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
          </Tabs.List>
          <Tabs.Panel value="general" keepMounted className="min-h-0 data-[hidden]:hidden">
            <div className="divide-y rounded-lg border">
              <SettingRow label={t("theme.title")}>
                <ModeToggle />
              </SettingRow>
              <SettingRow label={t("language.title")}>
                <LanguageToggle />
              </SettingRow>
              <SettingRow label={t("settings.notifications")}>
                <PwaControls />
              </SettingRow>
            </div>
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
