import { readFileSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vite-plus/test";
import {
  APP_SCOPES,
  type AppDetail,
  dirtySections,
  draftFromDetail,
  endpointCheck,
  eventsNeedingScope,
  needsReapproval,
  parseManifestJson,
  saveBody,
  toggleEvent,
  visibleSections,
} from "./model";

const catalog = {
  native_events: [
    { name: "message.created", scope: "messages:read", audience: "channel" },
    { name: "app.uninstalled", scope: null, audience: "app" },
  ],
};
const detail: AppDetail = {
  id: "app",
  name: "Echo",
  description: "Echo app",
  icon_url: null,
  bot_user_id: "bot",
  owner_id: "me",
  status: "installed",
  revision: 2,
  updated_at: "2026-11-07T00:00:00.123",
  manifest: {
    scopes: ["messages:write"],
    commands: ["deploy"],
    redirect_uris: ["https://example.com/callback"],
    transport: "http",
    event_url: "https://example.com/draft",
  },
  installation: {
    revision: 1,
    active: true,
    transport: "http",
    event_url: "https://example.com/approved",
    endpoint_verified_at: null,
  },
};

describe("app console model", () => {
  it("lists the same scopes as the API contract", () => {
    const api = JSON.parse(
      readFileSync(path.resolve(process.cwd(), "../docs/apps.openapi.json"), "utf8"),
    ) as {
      components: {
        schemas: { Manifest: { properties: { scopes: { items: { enum: string[] } } } } };
      };
    };
    expect([...APP_SCOPES]).toEqual(api.components.schemas.Manifest.properties.scopes.items.enum);
  });

  it("keeps settings from other sections when one section is saved", () => {
    const saved = draftFromDetail(detail, "echo_bot");
    const draft = {
      ...saved,
      manifest: toggleEvent(saved.manifest, catalog, "message.created", true),
    };
    const body = saveBody(draft, saved, detail.updated_at);
    expect(body.manifest.commands).toEqual(["deploy"]);
    expect(body.manifest.redirect_uris).toEqual(["https://example.com/callback"]);
    expect(body.manifest.events).toEqual(["message.created"]);
    expect(body.manifest.scopes).toEqual(["messages:write", "messages:read"]);
    expect(body.expected_updated_at).toBe(detail.updated_at);
    expect(body.bot_username).toBeNull();
    expect(dirtySections(saved, draft)).toEqual(["permissions", "events"]);
  });

  it("only asks for re-approval when the manifest changes", () => {
    const saved = draftFromDetail(detail, "echo_bot");
    expect(needsReapproval(saved, { ...saved, name: "Renamed", bot_username: "renamed" })).toBe(
      false,
    );
    expect(
      needsReapproval(saved, { ...saved, manifest: { ...saved.manifest, dm_enabled: true } }),
    ).toBe(true);
    expect(saveBody({ ...saved, bot_username: "renamed" }, saved, "t").bot_username).toBe(
      "renamed",
    );
  });

  it("checks the approved endpoint, not the draft URL", () => {
    const draft = draftFromDetail(detail);
    expect(endpointCheck(detail, draft)).toEqual({
      state: "ready",
      url: "https://example.com/approved",
      pendingUrl: "https://example.com/draft",
    });
    expect(endpointCheck({ ...detail, installation: null }, draft)).toEqual({
      state: "unapproved",
    });
    expect(
      endpointCheck(
        { ...detail, installation: { ...detail.installation!, transport: "websocket" } },
        draft,
      ),
    ).toEqual({ state: "not-http" });
  });

  it("shows incoming webhooks only for notification-only apps", () => {
    expect(visibleSections({ notification_only: false })).not.toContain("webhooks");
    expect(visibleSections({ notification_only: true })).toContain("webhooks");
  });

  it("reports events that depend on a scope and rejects malformed JSON", () => {
    const manifest = toggleEvent(
      draftFromDetail(detail).manifest,
      catalog,
      "message.created",
      true,
    );
    expect(eventsNeedingScope(manifest, catalog, "messages:read")).toEqual(["message.created"]);
    expect(parseManifestJson("[]")).toEqual({ ok: false });
    expect(parseManifestJson("{")).toEqual({ ok: false });
    const parsed = parseManifestJson('{"scopes":["messages:read"]}');
    expect(parsed.ok && parsed.manifest.transport).toBe("websocket");
  });

  it("rejects manifest fields of the wrong type or unknown keys", () => {
    expect(parseManifestJson('{"scopes":42}')).toEqual({ ok: false, field: "scopes" });
    expect(parseManifestJson('{"events":["a",1]}')).toEqual({ ok: false, field: "events" });
    expect(parseManifestJson('{"commands":null}')).toEqual({ ok: false, field: "commands" });
    expect(parseManifestJson('{"dm_enabled":"yes"}')).toEqual({ ok: false, field: "dm_enabled" });
    expect(parseManifestJson('{"transport":"ftp"}')).toEqual({ ok: false, field: "transport" });
    expect(parseManifestJson('{"scope":["messages:read"]}')).toEqual({ ok: false, field: "scope" });
    expect(parseManifestJson('{"toString":1}')).toEqual({ ok: false, field: "toString" });
    const parsed = parseManifestJson('{"event_url":null,"transport":"http","dm_enabled":true}');
    expect(parsed.ok && parsed.manifest).toMatchObject({
      event_url: null,
      transport: "http",
      dm_enabled: true,
      scopes: [],
    });
  });
});
