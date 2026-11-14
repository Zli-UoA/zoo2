import { ExternalLink } from "../ui/external-link";
import { useCallback, useEffect, useMemo, useState } from "react";
import { createConfiguredApiClient } from "../../lib/api/client";
import { useCollections } from "../../lib/electric/collections-context";
import { Textarea } from "../ui/textarea";
import { Input } from "../ui/input";
import { SettingsSection } from "../ui/control-panel";
import {
  ArrowLeftIcon,
  BellIcon,
  LanguagesIcon,
  LogOutIcon,
  PaletteIcon,
  SearchIcon,
  Volume2Icon,
} from "lucide-react";
import { Tabs } from "@base-ui/react/tabs";
import { useNavigate, useRouterState } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import { SearchEngineSelect } from "../preferences/SearchEngineSelect";
import { LanguageToggle } from "../i18n/LanguageToggle";
import { ModeToggle } from "../theme/ModeToggle";
import { PwaControls } from "../pwa/PwaControls";
import { AppsPanel } from "../chat/AppsDialog";
import { CallSettingsDevices } from "../call/CallSettingsDevices";
import { CallSoundsSwitch } from "../call/CallSoundsSwitch";
import { WorkspaceMembers } from "./WorkspaceMembers";
import { EncryptionSettings } from "../e2ee/EncryptionSettings";
import { DevicesSettings } from "../e2ee/DevicesSettings";
import { SlackImportPanel } from "./SlackImportPanel";
import { SlackLinkPanel } from "./SlackLinkPanel";
import { useCurrentUser } from "../../hooks/useCurrentUser";
import type { SettingsTab } from "../../routes/_shell.settings";
import { Button } from "../ui/button";
import { SettingRow, panelTabClassName } from "../ui/control-panel";
import { useShell } from "./shell-context";
import { joinUrl } from "../../lib/config";
import { ConfirmActionDialog } from "../ui/confirm-action-dialog";
import { PaneHeader } from "./PaneHeader";
import { LogoutButton } from "../auth/LogoutSetting";

const ADMIN_TABS: readonly SettingsTab[] = ["slack-import"];

export function ZooSettings() {
  const { t } = useTranslation();
  const shell = useShell();
  const navigate = useNavigate();
  const search = useRouterState({
    select: (state) => state.location.search as { tab?: SettingsTab; channel?: string },
  });
  const { config } = useCollections();
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
            <Tabs.Tab value="call" className={panelTabClassName}>
              {t("call.settingsTab")}
            </Tabs.Tab>
            <Tabs.Tab value="encryption" className={panelTabClassName}>
              {t("e2ee.tab")}
            </Tabs.Tab>
            <Tabs.Tab value="devices" className={panelTabClassName}>
              {t("e2ee.devicesTab")}
            </Tabs.Tab>
            <Tabs.Tab value="activitypub" className={panelTabClassName}>
              ActivityPub
            </Tabs.Tab>
            <Tabs.Tab value="emojis" className={panelTabClassName}>
              {t("home.customEmojis")}
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
            {config.oidc ? (
              <div className="mt-6 overflow-hidden rounded-lg border border-destructive/50">
                <SettingRow icon={<LogOutIcon />} label={t("auth.logout")}>
                  <LogoutButton />
                </SettingRow>
              </div>
            ) : null}
          </Tabs.Panel>
          <Tabs.Panel value="call" className="min-h-0">
            <h2 className="sr-only">{t("call.settingsTab")}</h2>
            <div className="divide-y rounded-lg border">
              <SettingRow
                icon={<Volume2Icon />}
                label={t("call.soundsLabel")}
                description={t("call.soundsDescription")}
              >
                <CallSoundsSwitch label={t("call.soundsLabel")} />
              </SettingRow>
            </div>
            <div className="mt-4 rounded-lg border p-4">
              <p className="mb-3 text-xs text-muted-foreground">{t("call.defaultsDescription")}</p>
              <CallSettingsDevices />
            </div>
          </Tabs.Panel>
          <Tabs.Panel value="encryption" className="min-h-0">
            <h2 className="sr-only">{t("e2ee.tab")}</h2>
            <EncryptionSettings />
          </Tabs.Panel>
          <Tabs.Panel value="devices" className="min-h-0">
            <h2 className="sr-only">{t("e2ee.devicesTab")}</h2>
            <DevicesSettings />
          </Tabs.Panel>
          <Tabs.Panel value="activitypub" className="min-h-0">
            <h2 className="sr-only">ActivityPub</h2>
            <ActivityPubSettings />
            <ChannelTimelineSettings />
          </Tabs.Panel>
          <Tabs.Panel value="emojis" className="min-h-0">
            <h2 className="sr-only">{t("home.customEmojis")}</h2>
            <CustomEmojiSettings />
          </Tabs.Panel>
          <Tabs.Panel value="apps" className="min-h-0">
            <AppsPanel channelId={search.channel} onClose={() => {}} />
          </Tabs.Panel>
          <Tabs.Panel value="members" className="min-h-0">
            <WorkspaceMembers />
          </Tabs.Panel>
          <Tabs.Panel value="slack" className="min-h-0">
            <h2 className="sr-only">{t("slackLink.tab")}</h2>
            <SlackLinkPanel />
          </Tabs.Panel>
          {isAdmin ? (
            <Tabs.Panel value="slack-import" className="min-h-0">
              <h2 className="sr-only">{t("slackImport.tab")}</h2>
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

function CustomEmojiSettings() {
  const { t } = useTranslation();
  const { config } = useCollections();
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  const [service, setService] = useState<{ url: string | null } | null>(null);
  const [failed, setFailed] = useState(false);
  const load = useCallback(async () => {
    setFailed(false);
    try {
      setService(await client.get<{ url: string | null }>("/api/emoji-service"));
    } catch {
      setFailed(true);
    }
  }, [client]);
  useEffect(() => {
    void load();
  }, [load]);
  if (service?.url === null) return <LocalCustomEmojiSettings />;
  return (
    <SettingsSection title={t("home.customEmojis")} description={t("customEmoji.serviceHint")}>
      {service?.url && /^https?:\/\//.test(service.url) ? (
        <ExternalLink href={service.url}>{t("customEmoji.manageService")}</ExternalLink>
      ) : null}
      {!service && !failed ? <p role="status">{t("home.loading")}</p> : null}
      {failed ? (
        <p role="alert">
          {t("home.controlsError")}{" "}
          <Button variant="link" onClick={() => void load()}>
            {t("home.refresh")}
          </Button>
        </p>
      ) : null}
    </SettingsSection>
  );
}

function LocalCustomEmojiSettings() {
  const { t } = useTranslation();
  const { config } = useCollections();
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  const [items, setItems] = useState<
    { name: string; url: string; category: string; aliases: string[] }[]
  >([]);
  const [name, setName] = useState("");
  const [category, setCategory] = useState("");
  const [aliases, setAliases] = useState("");
  const [previous, setPrevious] = useState<string | null>(null);
  const [remove, setRemove] = useState<string | null>(null);
  const [file, setFile] = useState<File | null>(null);
  const [fileVersion, setFileVersion] = useState(0);
  const [pending, setPending] = useState(false);
  const [failed, setFailed] = useState(false);
  const [loading, setLoading] = useState(true);
  const load = useCallback(async () => {
    setLoading(true);
    setFailed(false);
    try {
      setItems(await client.get<typeof items>("/api/emojis"));
    } catch {
      setFailed(true);
    } finally {
      setLoading(false);
    }
  }, [client]);
  useEffect(() => {
    void load();
  }, [load]);
  const reset = () => {
    setPrevious(null);
    setName("");
    setCategory("");
    setAliases("");
    setFile(null);
    setFileVersion((value) => value + 1);
  };
  return (
    <SettingsSection title={t("home.customEmojis")} description={t("customEmoji.hint")}>
      <form
        className="space-y-3"
        onSubmit={(event) => {
          event.preventDefault();
          if (pending || (!file && !previous)) return;
          setPending(true);
          setFailed(false);
          void (async () => {
            if (file) {
              if (file.size > 2 * 1024 * 1024) throw new Error("Image too large");
              const data = await new Promise<string>((resolve, reject) => {
                const reader = new FileReader();
                reader.onload = () =>
                  resolve(
                    (typeof reader.result === "string" ? reader.result : "").split(",")[1] ?? "",
                  );
                reader.onerror = reject;
                reader.readAsDataURL(file);
              });
              await client.request("/api/emojis/upload", {
                method: "POST",
                body: {
                  name,
                  category: category.trim(),
                  aliases: [
                    ...new Set(
                      aliases
                        .split(",")
                        .map((a) => a.trim())
                        .filter(Boolean),
                    ),
                  ],
                  previous_name: previous,
                  content_type: file.type,
                  data,
                },
              });
            } else {
              await client.request("/api/emojis", {
                method: "POST",
                body: {
                  name,
                  category: category.trim(),
                  aliases: [
                    ...new Set(
                      aliases
                        .split(",")
                        .map((a) => a.trim())
                        .filter(Boolean),
                    ),
                  ],
                  previous_name: previous,
                  url: items.find((e) => e.name === previous)?.url,
                },
              });
            }
            reset();
            window.dispatchEvent(new Event("workspace-emojis-changed"));
            await load();
          })()
            .catch(() => setFailed(true))
            .finally(() => setPending(false));
        }}
      >
        <label className="block space-y-1">
          <span className="block text-sm">{t("customEmoji.name")}</span>
          <Input
            className="w-full"
            value={name}
            onChange={(e) => setName(e.target.value)}
            pattern="[a-z0-9_-]+"
            maxLength={64}
            required
            disabled={pending}
          />
        </label>
        <label className="block space-y-1">
          <span className="block text-sm">{t("customEmoji.category")}</span>
          <Input
            className="w-full"
            value={category}
            onChange={(e) => setCategory(e.target.value)}
            maxLength={100}
            disabled={pending}
          />
        </label>
        <label className="block space-y-1">
          <span className="block text-sm">{t("customEmoji.aliases")}</span>
          <Input
            className="w-full"
            value={aliases}
            onChange={(e) => setAliases(e.target.value)}
            disabled={pending}
          />
        </label>
        <label className="block space-y-1">
          <span className="block text-sm">{t("customEmoji.image")}</span>
          <Input
            className="w-full"
            key={`${previous ?? "new"}:${fileVersion}`}
            type="file"
            accept="image/png,image/jpeg,image/gif,image/webp"
            onChange={(e) => setFile(e.target.files?.[0] ?? null)}
            required={!previous}
            disabled={pending}
          />
        </label>
        <Button type="submit" disabled={pending || !name || (!file && !previous)}>
          {t("home.save")}
        </Button>
        {previous ? (
          <Button type="button" variant="outline" disabled={pending} onClick={reset}>
            {t("customEmoji.cancel")}
          </Button>
        ) : null}
      </form>
      {loading ? <p role="status">{t("home.loading")}</p> : null}
      {failed ? (
        <p role="alert">
          {t("home.controlsError")}{" "}
          <Button variant="link" onClick={() => void load()}>
            {t("home.refresh")}
          </Button>
        </p>
      ) : null}
      {!loading && !items.length ? <p>{t("home.noEmojis")}</p> : null}
      <ul className="mt-4 divide-y">
        {items.map((item) => (
          <li key={item.name} className="flex flex-wrap items-center gap-3 py-3">
            <div className="flex min-w-0 basis-48 grow items-center gap-3">
              <img
                src={joinUrl(config.apiBaseUrl, item.url)}
                alt={`:${item.name}:`}
                className="h-8 w-auto max-w-24 shrink-0 object-contain"
              />
              <span className="min-w-0 flex-1 break-all">
                :{item.name}:
                <small className="block text-muted-foreground">
                  {item.category || t("customEmoji.uncategorized")}
                  {item.aliases?.length ? ` · ${item.aliases.join(", ")}` : ""}
                </small>
              </span>
            </div>
            <div className="ml-auto flex shrink-0 gap-2">
              <Button
                variant="outline"
                disabled={pending}
                onClick={() => {
                  setPrevious(item.name);
                  setName(item.name);
                  setCategory(item.category ?? "");
                  setAliases(item.aliases?.join(", ") ?? "");
                  setFile(null);
                  setFileVersion((value) => value + 1);
                }}
              >
                {t("customEmoji.edit")}
              </Button>
              <Button variant="outline" disabled={pending} onClick={() => setRemove(item.name)}>
                {t("customEmoji.remove")}
              </Button>
            </div>
          </li>
        ))}
      </ul>
      <ConfirmActionDialog
        open={remove !== null}
        onOpenChange={(open) => {
          if (!open) setRemove(null);
        }}
        title={t("customEmoji.remove")}
        description={`:${remove ?? ""}:`}
        confirmLabel={t("customEmoji.remove")}
        pending={pending}
        error={failed ? t("home.controlsError") : undefined}
        onConfirm={async () => {
          if (!remove) return;
          setPending(true);
          setFailed(false);
          try {
            await client.request(`/api/emojis/${encodeURIComponent(remove)}`, { method: "DELETE" });
            if (previous === remove) reset();
            setRemove(null);
            window.dispatchEvent(new Event("workspace-emojis-changed"));
            await load();
          } catch {
            setFailed(true);
          } finally {
            setPending(false);
          }
        }}
      />
    </SettingsSection>
  );
}
