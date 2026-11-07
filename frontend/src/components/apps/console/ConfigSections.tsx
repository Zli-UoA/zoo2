import { useEffect, useId, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Input } from "../../ui/input";
import { Textarea } from "../../ui/textarea";
import { Button } from "../../ui/button";
import { SettingsSection } from "../../ui/control-panel";
import { inputClassName } from "../../ui/input-styles";
import { useUsers } from "../../../hooks/useRealtime";
import type { SectionProps } from "./AppConsole";
import {
  APP_SCOPES,
  type AppManifest,
  endpointCheck,
  eventGroup,
  eventsNeedingScope,
  parseManifestJson,
  toggleEvent,
} from "./model";

function Checkbox({
  label,
  description,
  checked,
  disabled,
  onChange,
}: {
  label: string;
  description?: string;
  checked: boolean;
  disabled?: boolean;
  onChange: (checked: boolean) => void;
}) {
  return (
    <label className="flex items-start gap-2 text-sm">
      <Input
        type="checkbox"
        className="mt-0.5 size-4 shrink-0"
        checked={checked}
        disabled={disabled}
        onChange={(event) => onChange(event.target.checked)}
      />
      <span className="min-w-0">
        <span className="break-all">{label}</span>
        {description ? (
          <span className="block text-xs text-muted-foreground">{description}</span>
        ) : null}
      </span>
    </label>
  );
}

/** One entry per line; local text keeps partially typed lines until they are complete. */
function ListField({
  label,
  description,
  value,
  onChange,
}: {
  label: string;
  description?: string;
  value: string[];
  onChange: (value: string[]) => void;
}) {
  const [text, setText] = useState(value.join("\n"));
  const parse = (input: string) =>
    input
      .split("\n")
      .map((line) => line.trim())
      .filter(Boolean);
  const key = JSON.stringify(value);
  useEffect(() => {
    // Follow outside changes (discard, manifest JSON) without fighting the user's own typing.
    setText((current) => (JSON.stringify(parse(current)) === key ? current : value.join("\n")));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);
  return (
    <label className="flex flex-col gap-1 text-sm">
      {label}
      {description ? <span className="text-xs text-muted-foreground">{description}</span> : null}
      <Textarea
        className={`${inputClassName()} min-h-20 font-mono text-xs`}
        value={text}
        spellCheck={false}
        onChange={(event) => {
          setText(event.target.value);
          onChange(parse(event.target.value));
        }}
      />
    </label>
  );
}

function updateManifest(props: SectionProps, change: (manifest: AppManifest) => AppManifest) {
  props.setDraft((draft) => ({ ...draft, manifest: change(draft.manifest) }));
}

export function BasicSection(props: SectionProps) {
  const { t } = useTranslation();
  const { detail, draft, setDraft } = props;
  const people = useUsers().data ?? [];
  const field = (key: "name" | "description" | "icon_url" | "bot_username") => ({
    value: draft[key],
    onChange: (event: { target: { value: string } }) =>
      setDraft((current) => ({ ...current, [key]: event.target.value })),
  });
  return (
    <>
      <SettingsSection
        title={t("appConsole.basic.profile")}
        description={t("appConsole.basic.profileHelp")}
      >
        <label className="flex flex-col gap-1 text-sm">
          {t("apps.name")}
          <Input className={inputClassName()} required maxLength={80} {...field("name")} />
        </label>
        <label className="flex flex-col gap-1 text-sm">
          {t("appConsole.basic.username")}
          <Input
            className={inputClassName()}
            maxLength={80}
            spellCheck={false}
            {...field("bot_username")}
          />
        </label>
        <label className="flex flex-col gap-1 text-sm">
          {t("apps.about")}
          <Textarea className={inputClassName()} maxLength={4000} {...field("description")} />
        </label>
        <label className="flex flex-col gap-1 text-sm">
          {t("apps.icon")}
          <Input className={inputClassName()} type="url" {...field("icon_url")} />
        </label>
      </SettingsSection>
      <SettingsSection title={t("appConsole.basic.summary")}>
        <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-1 text-sm">
          <dt className="text-muted-foreground">{t("apps.developer")}</dt>
          <dd className="break-all">
            {people.find((user) => user.id === detail.owner_id)?.display_name ||
              t("apps.unknownDeveloper")}
          </dd>
          <dt className="text-muted-foreground">{t("appConsole.basic.status")}</dt>
          <dd>{t(`apps.status.${detail.status}`)}</dd>
          <dt className="text-muted-foreground">{t("appConsole.basic.revision")}</dt>
          <dd>
            {detail.revision}
            {detail.installation
              ? ` (${t("appConsole.basic.approved", { revision: detail.installation.revision })})`
              : ""}
          </dd>
          <dt className="text-muted-foreground">App ID</dt>
          <dd className="break-all font-mono text-xs">{detail.id}</dd>
        </dl>
      </SettingsSection>
    </>
  );
}

export function PermissionsSection(props: SectionProps) {
  const { t } = useTranslation();
  const { draft, catalog } = props;
  const { manifest } = draft;
  return (
    <>
      <SettingsSection
        title={t("appConsole.permissions.scopes")}
        description={t("appConsole.permissions.scopesHelp")}
      >
        <div className="grid gap-2 sm:grid-cols-2">
          {APP_SCOPES.map((scope) => {
            const needed = eventsNeedingScope(manifest, catalog, scope);
            const lockedByConnections = scope === "users:write" && manifest.user_connections;
            const lockedByFunctions =
              scope === "functions:execute" && manifest.functions.length > 0;
            const locked =
              manifest.scopes.includes(scope) &&
              (needed.length > 0 || lockedByConnections || lockedByFunctions);
            return (
              <Checkbox
                key={scope}
                label={scope}
                description={
                  locked
                    ? t("appConsole.permissions.requiredBy", {
                        names: [
                          ...needed,
                          ...(lockedByConnections ? ["user_connections"] : []),
                          ...(lockedByFunctions ? ["functions"] : []),
                        ].join(", "),
                      })
                    : undefined
                }
                checked={manifest.scopes.includes(scope)}
                disabled={locked}
                onChange={(checked) =>
                  updateManifest(props, (current) => ({
                    ...current,
                    scopes: checked
                      ? [...current.scopes, scope]
                      : current.scopes.filter((item) => item !== scope),
                  }))
                }
              />
            );
          })}
        </div>
      </SettingsSection>
      <SettingsSection title={t("appConsole.permissions.oauth")}>
        <ListField
          label={t("appConsole.permissions.redirectUris")}
          description={t("appConsole.listHelp")}
          value={manifest.redirect_uris}
          onChange={(redirect_uris) =>
            updateManifest(props, (current) => ({ ...current, redirect_uris }))
          }
        />
        <Checkbox
          label={t("appConsole.permissions.userConnections")}
          description={t("appConsole.permissions.userConnectionsHelp")}
          checked={manifest.user_connections}
          onChange={(user_connections) =>
            updateManifest(props, (current) => ({
              ...current,
              user_connections,
              scopes:
                user_connections && !current.scopes.includes("users:write")
                  ? [...current.scopes, "users:write"]
                  : current.scopes,
            }))
          }
        />
      </SettingsSection>
    </>
  );
}

export function EventsSection(props: SectionProps) {
  const { t } = useTranslation();
  const { draft, catalog } = props;
  const { manifest } = draft;
  const [filter, setFilter] = useState("");
  const groups = useMemo(() => {
    const result = new Map<string, { name: string; scope: string | null }[]>();
    for (const event of catalog?.native_events ?? []) {
      if (!event.name.includes(filter.trim().toLowerCase())) continue;
      const group = eventGroup(event.name);
      result.set(group, [...(result.get(group) ?? []), event]);
    }
    return [...result.entries()];
  }, [catalog, filter]);
  return (
    <>
      <SettingsSection title={t("appConsole.events.transport")}>
        <fieldset className="flex flex-wrap gap-4 text-sm">
          <legend className="sr-only">{t("apps.transport")}</legend>
          {(["websocket", "http"] as const).map((transport) => (
            <label key={transport} className="flex items-center gap-2">
              <Input
                type="radio"
                name="transport"
                className="size-4"
                checked={manifest.transport === transport}
                onChange={() => updateManifest(props, (current) => ({ ...current, transport }))}
              />
              {t(`appConsole.events.${transport}`)}
            </label>
          ))}
        </fieldset>
        {manifest.transport === "http" ? (
          <label className="flex flex-col gap-1 text-sm">
            {t("appConsole.events.requestUrl")}
            <Input
              className={inputClassName()}
              type="url"
              value={manifest.event_url ?? ""}
              placeholder="https://"
              onChange={(event) =>
                updateManifest(props, (current) => ({ ...current, event_url: event.target.value }))
              }
            />
          </label>
        ) : null}
        <Checkbox
          label={t("appConsole.events.includeBotEvents")}
          description={t("appConsole.events.includeBotEventsHelp")}
          checked={manifest.include_bot_events}
          onChange={(include_bot_events) =>
            updateManifest(props, (current) => ({ ...current, include_bot_events }))
          }
        />
      </SettingsSection>
      <EndpointVerification {...props} />
      <SettingsSection
        title={t("appConsole.events.subscriptions", { count: manifest.events.length })}
        description={t("appConsole.events.subscriptionsHelp")}
      >
        <Input
          className={inputClassName()}
          aria-label={t("appConsole.events.filter")}
          placeholder={t("appConsole.events.filter")}
          value={filter}
          onChange={(event) => setFilter(event.target.value)}
        />
        {!catalog ? (
          <p role="status" className="text-sm text-muted-foreground">
            {t("apps.loading")}
          </p>
        ) : null}
        {groups.map(([group, events]) => (
          <fieldset key={group} className="space-y-2">
            <legend className="text-xs font-semibold uppercase text-muted-foreground">
              {group}
            </legend>
            <div className="grid gap-2 sm:grid-cols-2">
              {events.map((event) => (
                <Checkbox
                  key={event.name}
                  label={event.name}
                  description={
                    event.scope ? t("appConsole.events.needs", { scope: event.scope }) : undefined
                  }
                  checked={manifest.events.includes(event.name)}
                  onChange={(checked) =>
                    updateManifest(props, (current) =>
                      toggleEvent(current, catalog, event.name, checked),
                    )
                  }
                />
              ))}
            </div>
          </fieldset>
        ))}
      </SettingsSection>
    </>
  );
}

/** Checks the approved installation's URL; the draft URL is only checkable after re-approval. */
function EndpointVerification({ detail, draft, client, refresh }: SectionProps) {
  const { t, i18n } = useTranslation();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(false);
  const check = endpointCheck(detail, draft);
  const verifiedAt = detail.installation?.endpoint_verified_at;
  return (
    <SettingsSection
      title={t("appConsole.events.verifyTitle")}
      description={t("appConsole.events.verifyHelp")}
    >
      {check.state === "unapproved" ? (
        <p className="text-sm text-muted-foreground">
          {t("appConsole.events.verifyAfterApproval")}
        </p>
      ) : check.state === "not-http" ? (
        <p className="text-sm text-muted-foreground">{t("appConsole.events.verifyHttpOnly")}</p>
      ) : (
        <>
          <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-1 text-sm">
            <dt className="text-muted-foreground">{t("appConsole.events.approvedUrl")}</dt>
            <dd className="break-all font-mono text-xs">{check.url}</dd>
            <dt className="text-muted-foreground">{t("appConsole.events.verifiedAt")}</dt>
            <dd>
              {verifiedAt
                ? new Date(verifiedAt).toLocaleString(i18n.language)
                : t("appConsole.events.notVerified")}
            </dd>
          </dl>
          {check.pendingUrl ? (
            <p className="text-xs text-muted-foreground">
              {t("appConsole.events.pendingUrl", { url: check.pendingUrl })}
            </p>
          ) : null}
          <div>
            <Button
              variant="outline"
              disabled={busy}
              onClick={() => {
                setBusy(true);
                setError(false);
                client
                  .request(`/api/apps/${encodeURIComponent(detail.id)}/actions/verify-endpoint`, {
                    method: "POST",
                  })
                  .then(refresh)
                  .catch(() => setError(true))
                  .finally(() => setBusy(false));
              }}
            >
              {t("appConsole.events.verify")}
            </Button>
          </div>
          {error ? (
            <p role="alert" className="text-sm text-destructive">
              {t("appConsole.events.verifyError")}
            </p>
          ) : null}
        </>
      )}
    </SettingsSection>
  );
}

export function InteractivitySection(props: SectionProps) {
  const { t } = useTranslation();
  const { manifest } = props.draft;
  const list = (
    key: "commands" | "shortcuts" | "message_shortcuts" | "link_domains" | "functions",
  ) => (
    <ListField
      key={key}
      label={t(`appConsole.interactivity.${key}`)}
      description={t(`appConsole.interactivity.${key}Help`)}
      value={manifest[key]}
      onChange={(value) =>
        updateManifest(props, (current) => ({
          ...current,
          [key]: value,
          scopes:
            key === "functions" && value.length && !current.scopes.includes("functions:execute")
              ? [...current.scopes, "functions:execute"]
              : current.scopes,
        }))
      }
    />
  );
  return (
    <>
      <SettingsSection title={t("appConsole.interactivity.commandsTitle")}>
        {list("commands")}
      </SettingsSection>
      <SettingsSection title={t("appConsole.interactivity.shortcutsTitle")}>
        {list("shortcuts")}
        {list("message_shortcuts")}
      </SettingsSection>
      <SettingsSection title={t("appConsole.interactivity.moreTitle")}>
        {list("link_domains")}
        {list("functions")}
      </SettingsSection>
    </>
  );
}

export function HomeSection(props: SectionProps) {
  const { t } = useTranslation();
  return (
    <SettingsSection title={t("appConsole.home.title")} description={t("appConsole.home.help")}>
      <Checkbox
        label={t("appConsole.home.dm")}
        description={t("appConsole.home.dmHelp")}
        checked={props.draft.manifest.dm_enabled}
        onChange={(dm_enabled) => updateManifest(props, (current) => ({ ...current, dm_enabled }))}
      />
    </SettingsSection>
  );
}

export function ManifestSection(props: SectionProps) {
  const { t } = useTranslation();
  const id = useId();
  const formatted = JSON.stringify(props.draft.manifest, null, 2);
  const [text, setText] = useState(formatted);
  // `field` is set when the JSON parsed but one key is unknown or has the wrong type.
  const [invalid, setInvalid] = useState<{ field?: string } | null>(null);
  // Follow edits made in other sections. Adjusting during render (not in an effect) avoids
  // overwriting text typed before a deferred mount effect runs.
  const [shown, setShown] = useState(formatted);
  if (shown !== formatted) {
    setShown(formatted);
    setText(formatted);
    setInvalid(null);
  }
  return (
    <SettingsSection title={t("apps.manifest")} description={t("appConsole.manifest.help")}>
      <Textarea
        id={id}
        aria-label={t("apps.manifest")}
        aria-invalid={invalid !== null}
        className={`${inputClassName()} min-h-80 font-mono text-xs`}
        spellCheck={false}
        value={text}
        onChange={(event) => setText(event.target.value)}
      />
      {invalid ? (
        <p role="alert" className="text-sm text-destructive">
          {invalid.field
            ? t("appConsole.manifest.invalidField", { field: invalid.field })
            : t("apps.ui.invalidManifest")}
        </p>
      ) : null}
      <div className="flex justify-end gap-2">
        <Button variant="ghost" disabled={text === formatted} onClick={() => setText(formatted)}>
          {t("appConsole.manifest.reset")}
        </Button>
        <Button
          variant="outline"
          disabled={text === formatted}
          onClick={() => {
            const parsed = parseManifestJson(text);
            if (!parsed.ok) {
              setInvalid({ field: parsed.field });
              return;
            }
            setInvalid(null);
            props.setDraft((draft) => ({ ...draft, manifest: parsed.manifest }));
          }}
        >
          {t("appConsole.manifest.apply")}
        </Button>
      </div>
    </SettingsSection>
  );
}
