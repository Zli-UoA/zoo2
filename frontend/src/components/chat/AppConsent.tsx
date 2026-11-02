import { Input } from "../ui/input";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { createConfiguredApiClient } from "../../lib/api/client";
import { useCollections } from "../../lib/electric/collections-context";
import { useUsers } from "../../hooks/useRealtime";
import { Button } from "../ui/button";
import { AppAvatar } from "./AppIdentity";

export function AppConsent() {
  const { t } = useTranslation();
  const { config } = useCollections();
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  const params = useMemo(() => new URLSearchParams(window.location.search), []);
  const people = useUsers().data ?? [];
  const id = params.get("app_id");
  const requested = useMemo(() => (params.get("scope") ?? "").split(" ").filter(Boolean), [params]);
  const [app, setApp] = useState<{
    name: string;
    description: string;
    icon_url: string | null;
    owner_id: string;
    manifest: { scopes?: string[]; redirect_uris?: string[] };
  }>();
  const [selected, setSelected] = useState(requested);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => {
    const controller = new AbortController();
    if (!id) {
      setError(t("apps.actionError"));
      return;
    }
    client
      .get<typeof app>(`/api/apps/${encodeURIComponent(id)}`, { signal: controller.signal })
      .then(setApp)
      .catch(() => {
        if (!controller.signal.aborted) setError(t("apps.loadError"));
      });
    return () => controller.abort();
  }, [client, id, t]);
  const valid =
    app &&
    params.get("code_challenge_method") === "S256" &&
    !!params.get("state") &&
    !!params.get("code_challenge") &&
    app.manifest.redirect_uris?.includes(params.get("redirect_uri") ?? "") &&
    requested.every((scope) => app.manifest.scopes?.includes(scope));
  async function consent() {
    if (!valid || busy) return;
    setBusy(true);
    try {
      const result = await client.request<{ redirect_uri: string }>("/api/apps/oauth/authorize", {
        method: "POST",
        body: {
          app_id: id,
          redirect_uri: params.get("redirect_uri"),
          code_challenge: params.get("code_challenge"),
          state: params.get("state"),
          scopes: selected,
        },
      });
      window.location.assign(result.redirect_uri);
    } catch {
      setError(t("apps.actionError"));
      setBusy(false);
    }
  }
  return (
    <div className="mx-auto w-full max-w-lg space-y-4 overflow-y-auto p-6">
      <h1 className="text-xl font-semibold">{t("apps.consentTitle")}</h1>
      {app ? (
        <>
          <div className="flex items-center gap-3">
            <AppAvatar label={app.name} isBot src={app.icon_url} />
            <h2>{app.name}</h2>
          </div>
          <p>{app.description}</p>
          <p className="break-all text-xs">
            {t("apps.developer")}:{" "}
            {people.find((user) => user.id === app.owner_id)?.display_name ||
              t("apps.unknownDeveloper")}
          </p>
          <p className="text-sm">{t("apps.consentDescription")}</p>
          <ul className="space-y-2">
            {requested.map((scope) => (
              <li key={scope}>
                <label className="flex items-center gap-2 text-sm">
                  <Input
                    type="checkbox"
                    checked={selected.includes(scope)}
                    onChange={(event) =>
                      setSelected(
                        event.target.checked
                          ? [...selected, scope]
                          : selected.filter((s) => s !== scope),
                      )
                    }
                  />
                  {scope}
                </label>
              </li>
            ))}
          </ul>
          <p className="break-all text-xs">{params.get("redirect_uri")}</p>
        </>
      ) : null}
      {error || (app && !valid) ? (
        <p role="alert" className="text-destructive">
          {error || t("apps.actionError")}
        </p>
      ) : null}
      <div className="flex gap-2">
        <Button disabled={!valid || busy} onClick={() => void consent()}>
          {t("apps.allow")}
        </Button>
        <Button variant="outline" onClick={() => window.location.assign("/")}>
          {t("apps.cancel")}
        </Button>
      </div>
    </div>
  );
}
