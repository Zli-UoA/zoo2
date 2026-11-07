/** Mirrors the backend `Manifest`; the server rejects unknown fields and fills omitted ones with defaults. */
export type AppManifest = {
  scopes: string[];
  events: string[];
  notification_only: boolean;
  include_bot_events: boolean;
  dm_enabled: boolean;
  transport: "websocket" | "http";
  event_url: string | null;
  redirect_uris: string[];
  commands: string[];
  shortcuts: string[];
  message_shortcuts: string[];
  link_domains: string[];
  functions: string[];
  user_connections: boolean;
};

export type AppStatus = "draft" | "pending" | "installed" | "disabled";

/** The approved configuration that deliveries and endpoint checks use until the next approval. */
export type AppInstallation = {
  revision: number;
  active: boolean;
  transport: string | null;
  event_url: string | null;
  endpoint_verified_at: string | null;
};

export type AppDetail = {
  id: string;
  name: string;
  description: string;
  icon_url: string | null;
  bot_user_id: string;
  owner_id: string;
  status: AppStatus;
  revision: number;
  updated_at: string;
  manifest: Partial<AppManifest>;
  installation?: AppInstallation | null;
};

/** The whole PATCH body. Every save sends all of it so one section cannot erase another. */
export type AppDraft = {
  name: string;
  description: string;
  icon_url: string;
  bot_username: string;
  manifest: AppManifest;
};

export type EventCatalog = {
  native_events: { name: string; scope: string | null; audience: string }[];
};

/** Keep in sync with `Manifest.scopes` in docs/apps.openapi.json (checked by model.test.ts). */
export const APP_SCOPES = [
  "messages:read",
  "messages:write",
  "reactions:write",
  "channels:read",
  "channels:manage",
  "users:read",
  "dm:write",
  "files:read",
  "files:write",
  "ui:write",
  "commands:read",
  "links:write",
  "users:write",
  "preferences:read",
  "preferences:write",
  "pins:read",
  "pins:write",
  "stars:read",
  "stars:write",
  "emoji:read",
  "emoji:write",
  "usergroups:read",
  "usergroups:write",
  "workspace:read",
  "workspace:write",
  "apps:read",
  "invites:read",
  "invites:write",
  "reminders:read",
  "reminders:write",
  "files:share",
  "metadata:read",
  "metadata:write",
  "assistant:write",
  "functions:execute",
] as const;

export const CONSOLE_SECTIONS = [
  "basic",
  "credentials",
  "permissions",
  "events",
  "webhooks",
  "interactivity",
  "home",
  "install",
  "deliveries",
  "history",
  "manifest",
] as const;
export type ConsoleSection = (typeof CONSOLE_SECTIONS)[number];

export function parseSection(value: unknown): ConsoleSection {
  return CONSOLE_SECTIONS.includes(value as ConsoleSection) ? (value as ConsoleSection) : "basic";
}

/** Incoming webhooks belong to notification-only apps; regular apps do not show the section. */
export function visibleSections(manifest: Pick<AppManifest, "notification_only">) {
  return CONSOLE_SECTIONS.filter((section) => section !== "webhooks" || manifest.notification_only);
}

export function normalizeManifest(value: Partial<AppManifest> = {}): AppManifest {
  return {
    scopes: value.scopes ?? [],
    events: value.events ?? [],
    notification_only: value.notification_only ?? false,
    include_bot_events: value.include_bot_events ?? false,
    dm_enabled: value.dm_enabled ?? false,
    transport: value.transport === "http" ? "http" : "websocket",
    event_url: value.event_url ?? null,
    redirect_uris: value.redirect_uris ?? [],
    commands: value.commands ?? [],
    shortcuts: value.shortcuts ?? [],
    message_shortcuts: value.message_shortcuts ?? [],
    link_domains: value.link_domains ?? [],
    functions: value.functions ?? [],
    user_connections: value.user_connections ?? false,
  };
}

export function draftFromDetail(detail: AppDetail, botUsername = ""): AppDraft {
  return {
    name: detail.name,
    description: detail.description,
    icon_url: detail.icon_url ?? "",
    bot_username: botUsername,
    manifest: normalizeManifest(detail.manifest),
  };
}

export function saveBody(draft: AppDraft, saved: AppDraft, expectedUpdatedAt: string) {
  return {
    name: draft.name.trim(),
    description: draft.description,
    icon_url: draft.icon_url.trim() || null,
    // Omitting the username keeps the current one; send it only when it changed.
    bot_username:
      draft.bot_username.trim() && draft.bot_username !== saved.bot_username
        ? draft.bot_username.trim()
        : null,
    manifest: {
      ...draft.manifest,
      event_url: draft.manifest.event_url?.trim() || null,
    },
    expected_updated_at: expectedUpdatedAt,
  };
}

function same(a: unknown, b: unknown) {
  return JSON.stringify(a) === JSON.stringify(b);
}

/** Manifest keys whose values differ; any of them makes the server start a new revision. */
export function changedManifestKeys(saved: AppManifest, draft: AppManifest) {
  return (Object.keys(draft) as (keyof AppManifest)[]).filter(
    (key) => !same(saved[key], draft[key]),
  );
}

export function isDirty(saved: AppDraft, draft: AppDraft) {
  return !same(saved, draft);
}

/** Profile-only edits keep the revision and approval; manifest edits need re-approval. */
export function needsReapproval(saved: AppDraft, draft: AppDraft) {
  return changedManifestKeys(saved.manifest, draft.manifest).length > 0;
}

/** Sections with unsaved edits, so a save from one section can say what else it includes. */
export function dirtySections(saved: AppDraft, draft: AppDraft): ConsoleSection[] {
  const sections = new Set<ConsoleSection>();
  if (
    saved.name !== draft.name ||
    saved.description !== draft.description ||
    saved.icon_url !== draft.icon_url ||
    saved.bot_username !== draft.bot_username
  )
    sections.add("basic");
  for (const key of changedManifestKeys(saved.manifest, draft.manifest)) {
    sections.add(manifestSection[key]);
  }
  return CONSOLE_SECTIONS.filter((section) => sections.has(section));
}

const manifestSection: Record<keyof AppManifest, ConsoleSection> = {
  scopes: "permissions",
  redirect_uris: "permissions",
  user_connections: "permissions",
  events: "events",
  transport: "events",
  event_url: "events",
  include_bot_events: "events",
  commands: "interactivity",
  shortcuts: "interactivity",
  message_shortcuts: "interactivity",
  link_domains: "interactivity",
  functions: "interactivity",
  dm_enabled: "home",
  notification_only: "manifest",
};

export function requiredScope(catalog: EventCatalog | null, event: string) {
  return catalog?.native_events.find((entry) => entry.name === event)?.scope ?? null;
}

/** Subscribing to an event also requests the scope it needs, as the server would reject it otherwise. */
export function toggleEvent(
  manifest: AppManifest,
  catalog: EventCatalog | null,
  event: string,
  enabled: boolean,
): AppManifest {
  if (!enabled) return { ...manifest, events: manifest.events.filter((name) => name !== event) };
  const scope = requiredScope(catalog, event);
  return {
    ...manifest,
    events: manifest.events.includes(event) ? manifest.events : [...manifest.events, event],
    scopes:
      scope && !manifest.scopes.includes(scope) ? [...manifest.scopes, scope] : manifest.scopes,
  };
}

/** Events that would stop validating if this scope were removed. */
export function eventsNeedingScope(
  manifest: AppManifest,
  catalog: EventCatalog | null,
  scope: string,
) {
  return manifest.events.filter((event) => requiredScope(catalog, event) === scope);
}

export function eventGroup(name: string) {
  return name.split(".")[0] ?? name;
}

/** Whether the approved endpoint can be checked, and why not otherwise. */
export function endpointCheck(detail: AppDetail, draft: AppDraft) {
  const installation = detail.installation;
  if (!installation?.active) return { state: "unapproved" as const };
  if (installation.transport !== "http" || !installation.event_url)
    return { state: "not-http" as const };
  const draftUrl = draft.manifest.transport === "http" ? draft.manifest.event_url : null;
  return {
    state: "ready" as const,
    url: installation.event_url,
    pendingUrl: draftUrl && draftUrl !== installation.event_url ? draftUrl : null,
  };
}

export type ManifestParse = { ok: true; manifest: AppManifest } | { ok: false };

export function parseManifestJson(text: string): ManifestParse {
  try {
    const value: unknown = JSON.parse(text);
    if (!value || typeof value !== "object" || Array.isArray(value)) return { ok: false };
    return { ok: true, manifest: normalizeManifest(value as Partial<AppManifest>) };
  } catch {
    return { ok: false };
  }
}

/** Top-level keys that differ between two saved revisions, for the history view. */
export function revisionChanges(
  previous: Partial<AppManifest> | undefined,
  next: Partial<AppManifest>,
) {
  if (!previous) return [];
  return changedManifestKeys(normalizeManifest(previous), normalizeManifest(next));
}
