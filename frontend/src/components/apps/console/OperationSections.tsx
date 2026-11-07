import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "@tanstack/react-router";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";
import { Select } from "../../ui/select";
import { ConfirmActionDialog } from "../../ui/confirm-action-dialog";
import { SettingsSection } from "../../ui/control-panel";
import { inputClassName } from "../../ui/input-styles";
import { describeApiError } from "../../../hooks/useChannelAdmin";
import { useCurrentUser } from "../../../hooks/useCurrentUser";
import { useActiveChannels, useUsers } from "../../../hooks/useRealtime";
import { AppCredentials } from "../../chat/AppSettingsSections";
import { type ChannelWebhook, WebhookItem } from "../../chat/ChannelWebhooksDialog";
import type { SectionProps } from "./AppConsole";
import { type AppManifest, revisionChanges } from "./model";

function useAppResource<T>(props: SectionProps, path: string, initial: T) {
  const { client } = props;
  const [value, setValue] = useState<T>(initial);
  const [loading, setLoading] = useState(true);
  const [failed, setFailed] = useState(false);
  const [version, setVersion] = useState(0);
  useEffect(() => {
    const controller = new AbortController();
    setLoading(true);
    setFailed(false);
    client
      .get<T>(path, { signal: controller.signal })
      .then((next) => {
        if (!controller.signal.aborted) setValue(next);
      })
      .catch(() => {
        if (!controller.signal.aborted) setFailed(true);
      })
      .finally(() => {
        if (!controller.signal.aborted) setLoading(false);
      });
    return () => controller.abort();
  }, [client, path, version]);
  const reload = useCallback(() => setVersion((current) => current + 1), []);
  return { value, setValue, loading, failed, reload };
}

function LoadState({
  loading,
  failed,
  onRetry,
}: {
  loading: boolean;
  failed: boolean;
  onRetry: () => void;
}) {
  const { t } = useTranslation();
  if (loading)
    return (
      <p role="status" className="text-sm text-muted-foreground">
        {t("apps.loading")}
      </p>
    );
  if (!failed) return null;
  return (
    <div role="alert" className="flex flex-wrap items-center gap-2 text-sm">
      <span className="text-destructive">{t("appConsole.loadError")}</span>
      <Button variant="outline" size="sm" onClick={onRetry}>
        {t("offline.retry")}
      </Button>
    </div>
  );
}

function useAction() {
  const { t } = useTranslation();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const run = useCallback(
    async (action: () => Promise<void>) => {
      if (busy) return;
      setBusy(true);
      setError("");
      try {
        await action();
      } catch (cause) {
        setError(describeApiError(cause, t));
      } finally {
        setBusy(false);
      }
    },
    [busy, t],
  );
  return { busy, error, setError, run };
}

function ErrorText({ error }: { error: string }) {
  return error ? (
    <p role="alert" className="text-sm text-destructive">
      {error}
    </p>
  ) : null;
}

function formatTime(value: string | null | undefined, locale: string) {
  return value ? new Date(value).toLocaleString(locale) : "";
}

type CredentialRecord = {
  id: string;
  user_id: string | null;
  scopes: string[];
  expires_at: string | null;
  revoked_at: string | null;
};

export function CredentialsSection(props: SectionProps) {
  const { t, i18n } = useTranslation();
  const { detail, client } = props;
  const base = `/api/apps/${encodeURIComponent(detail.id)}`;
  const { busy, error, run } = useAction();
  const [token, setToken] = useState("");
  const [secret, setSecret] = useState("");
  const [confirmRotate, setConfirmRotate] = useState(false);
  const records = useAppResource<CredentialRecord[]>(props, `${base}/records/credentials`, []);
  const installed = detail.status === "installed" && Boolean(detail.installation?.active);
  const http = detail.installation?.transport === "http";
  return (
    <>
      <SettingsSection
        title={t("appConsole.credentials.botToken")}
        description={t("appConsole.credentials.botTokenHelp")}
      >
        {installed && !detail.manifest.notification_only ? (
          <Button
            variant="outline"
            disabled={busy}
            onClick={() =>
              void run(async () => {
                const result = await client.request<Record<string, unknown>>(
                  `${base}/actions/token`,
                  { method: "POST" },
                );
                setToken(JSON.stringify(result, null, 2));
                records.reload();
              })
            }
          >
            {t("apps.issueToken")}
          </Button>
        ) : (
          <p className="text-sm text-muted-foreground">
            {t("appConsole.credentials.needsInstall")}
          </p>
        )}
        {token ? (
          <>
            <AppCredentials value={token} onError={() => undefined} />
            <p className="text-xs text-muted-foreground">{t("appConsole.credentials.tokenOnce")}</p>
          </>
        ) : null}
      </SettingsSection>
      <SettingsSection
        title={t("appConsole.credentials.signingSecret")}
        description={t("appConsole.credentials.signingSecretHelp")}
      >
        {installed && http ? (
          <>
            <div className="flex flex-wrap gap-2">
              <Button
                variant="outline"
                disabled={busy}
                onClick={() =>
                  void run(async () => {
                    const result = await client.request<{ signing_secret: string }>(
                      `${base}/actions/connection`,
                      { method: "POST" },
                    );
                    setSecret(result.signing_secret);
                  })
                }
              >
                {t("appConsole.credentials.showSecret")}
              </Button>
              <Button variant="outline" disabled={busy} onClick={() => setConfirmRotate(true)}>
                {t("appConsole.credentials.rotateSecret")}
              </Button>
            </div>
            <p className="text-xs text-muted-foreground">
              {t("appConsole.credentials.secretAudited")}
            </p>
            {secret ? <AppCredentials value={secret} onError={() => undefined} /> : null}
            <ConfirmActionDialog
              open={confirmRotate}
              onOpenChange={setConfirmRotate}
              title={t("appConsole.credentials.rotateSecret")}
              description={t("appConsole.credentials.rotateHelp")}
              confirmLabel={t("appConsole.credentials.rotateSecret")}
              pending={busy}
              error={error}
              onConfirm={() =>
                run(async () => {
                  const result = await client.request<{ signing_secret: string }>(
                    `${base}/actions/rotate-signing-secret`,
                    { method: "POST" },
                  );
                  setSecret(result.signing_secret);
                  setConfirmRotate(false);
                  props.refresh();
                })
              }
            />
          </>
        ) : (
          <p className="text-sm text-muted-foreground">
            {t("appConsole.credentials.secretHttpOnly")}
          </p>
        )}
      </SettingsSection>
      <ErrorText error={confirmRotate ? "" : error} />
      <SettingsSection title={t("appConsole.credentials.issued")}>
        <LoadState loading={records.loading} failed={records.failed} onRetry={records.reload} />
        {!records.loading && !records.failed && records.value.length === 0 ? (
          <p className="text-sm text-muted-foreground">{t("appConsole.credentials.none")}</p>
        ) : null}
        <ul className="divide-y rounded-lg border empty:hidden">
          {records.value.map((record) => {
            const state = record.revoked_at
              ? "revoked"
              : record.expires_at && new Date(record.expires_at) < new Date()
                ? "expired"
                : "active";
            return (
              <li key={record.id} className="flex flex-wrap items-center gap-2 p-3 text-xs">
                <span className="min-w-0 flex-1 space-y-0.5">
                  <span className="block font-mono">{record.id.slice(0, 8)}</span>
                  <span className="block break-all text-muted-foreground">
                    {record.user_id
                      ? t("appConsole.credentials.userToken")
                      : t("appConsole.credentials.botToken")}{" "}
                    · {record.scopes.join(", ")}
                  </span>
                  <span className="block text-muted-foreground">
                    {t(`appConsole.credentials.state.${state}`)}
                    {state === "active" && record.expires_at
                      ? ` · ${t("appConsole.credentials.expires", { time: formatTime(record.expires_at, i18n.language) })}`
                      : ""}
                  </span>
                </span>
                {state === "active" ? (
                  <Button
                    variant="ghost"
                    size="sm"
                    disabled={busy}
                    onClick={() =>
                      void run(async () => {
                        await client.request(
                          `${base}/credentials/${encodeURIComponent(record.id)}`,
                          { method: "DELETE" },
                        );
                        records.reload();
                      })
                    }
                  >
                    {t("appConsole.credentials.revoke")}
                  </Button>
                ) : null}
              </li>
            );
          })}
        </ul>
      </SettingsSection>
    </>
  );
}

export function InstallSection(props: SectionProps) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { detail, client, isAdmin, refresh } = props;
  const base = `/api/apps/${encodeURIComponent(detail.id)}`;
  const { busy, error, setError, run } = useAction();
  const [confirm, setConfirm] = useState<"disable" | "delete" | null>(null);
  const act = (kind: string, body?: unknown) =>
    run(async () => {
      await client.request(`${base}/actions/${kind}`, { method: "POST", body });
      refresh();
    });
  const stopped = detail.status === "draft" && detail.installation?.active;
  return (
    <>
      <SettingsSection
        title={t("appConsole.install.approval")}
        description={t("appConsole.install.approvalHelp")}
      >
        <p className="text-sm">
          {t(`apps.status.${detail.status}`)}
          {stopped ? ` · ${t("appConsole.install.stopped")}` : ""}
        </p>
        <div className="flex flex-wrap gap-2">
          {detail.status === "draft" || detail.status === "disabled" ? (
            <Button variant="outline" disabled={busy} onClick={() => void act("submit")}>
              {t("apps.requestApproval")}
            </Button>
          ) : null}
          {isAdmin && detail.status === "pending" ? (
            <Button
              disabled={busy}
              onClick={() => void act("install", { revision: detail.revision })}
            >
              {t("appConsole.install.approve", { revision: detail.revision })}
            </Button>
          ) : null}
          {detail.status !== "disabled" ? (
            <Button
              variant="destructive"
              disabled={busy}
              onClick={() => {
                setError("");
                setConfirm("disable");
              }}
            >
              {t("apps.disable")}
            </Button>
          ) : null}
        </div>
        {detail.status === "pending" && !isAdmin ? (
          <p className="text-xs text-muted-foreground">{t("appConsole.install.waiting")}</p>
        ) : null}
        <ErrorText error={confirm ? "" : error} />
      </SettingsSection>
      <SettingsSection
        title={t("appConsole.install.consent")}
        description={t("appConsole.install.consentHelp")}
      >
        <div>
          <Button
            variant="ghost"
            disabled={busy}
            onClick={() =>
              void run(async () => {
                await client.request(`${base}/authorization`, { method: "DELETE" });
              })
            }
          >
            {t("apps.revokeConsent")}
          </Button>
        </div>
      </SettingsSection>
      <SettingsSection
        title={t("apps.ui.delete")}
        description={t("apps.ui.deleteHelp")}
        className="border-destructive/30"
      >
        <div>
          <Button
            variant="destructive"
            disabled={busy}
            onClick={() => {
              setError("");
              setConfirm("delete");
            }}
          >
            {t("apps.ui.delete")}
          </Button>
        </div>
      </SettingsSection>
      <ConfirmActionDialog
        open={confirm !== null}
        onOpenChange={(open) => {
          if (!open) setConfirm(null);
        }}
        title={
          confirm === "delete" ? t("apps.ui.deleteTitle", { name: detail.name }) : t("apps.disable")
        }
        description={confirm === "delete" ? t("apps.ui.deleteHelp") : t("apps.ui.disableHelp")}
        confirmLabel={confirm === "delete" ? t("apps.ui.delete") : t("apps.disable")}
        pending={busy}
        error={error}
        onConfirm={() =>
          run(async () => {
            await client.request(`${base}/actions/${confirm}`, { method: "POST" });
            if (confirm === "delete") {
              await navigate({ to: "/apps" });
              return;
            }
            setConfirm(null);
            refresh();
          })
        }
      />
    </>
  );
}

type Delivery = {
  id: string;
  event_id: string;
  type: string;
  attempts: number;
  acked_at: string | null;
  last_error: string | null;
  created_at: string;
};

export function DeliveriesSection(props: SectionProps) {
  const { t, i18n } = useTranslation();
  const base = `/api/apps/${encodeURIComponent(props.detail.id)}`;
  const deliveries = useAppResource<Delivery[]>(props, `${base}/deliveries`, []);
  const { busy, error, run } = useAction();
  return (
    <SettingsSection title={t("apps.deliveries")} description={t("appConsole.deliveries.help")}>
      <div>
        <Button
          variant="outline"
          size="sm"
          disabled={deliveries.loading}
          onClick={deliveries.reload}
        >
          {t("appConsole.reload")}
        </Button>
      </div>
      <LoadState
        loading={deliveries.loading}
        failed={deliveries.failed}
        onRetry={deliveries.reload}
      />
      <ErrorText error={error} />
      {!deliveries.loading && !deliveries.failed && deliveries.value.length === 0 ? (
        <p className="text-sm text-muted-foreground">{t("appConsole.deliveries.none")}</p>
      ) : null}
      <ul className="divide-y rounded-lg border empty:hidden">
        {deliveries.value.map((delivery) => (
          <li key={delivery.id} className="flex flex-wrap items-center gap-2 p-3 text-xs">
            <span className="min-w-0 flex-1 space-y-0.5">
              <span className="block font-mono">{delivery.type}</span>
              <span className="block text-muted-foreground">
                {formatTime(delivery.created_at, i18n.language)} ·{" "}
                {t("appConsole.deliveries.attempts", { count: delivery.attempts })} ·{" "}
                {delivery.acked_at ? t("apps.delivered") : t("apps.pending")}
              </span>
              {!delivery.acked_at && delivery.last_error ? (
                <span className="block break-all text-destructive">{delivery.last_error}</span>
              ) : null}
            </span>
            {!delivery.acked_at ? (
              <Button
                variant="ghost"
                size="sm"
                disabled={busy}
                onClick={() =>
                  void run(async () => {
                    await props.client.request(
                      `${base}/deliveries/${encodeURIComponent(delivery.id)}/retry`,
                      { method: "POST" },
                    );
                    deliveries.reload();
                  })
                }
              >
                {t("apps.retry")}
              </Button>
            ) : null}
          </li>
        ))}
      </ul>
    </SettingsSection>
  );
}

type Version = { revision: number; manifest: Partial<AppManifest>; created_at: string };
type AuditEntry = { id: string; actor_id: string; action: string; created_at: string };

export function HistorySection(props: SectionProps) {
  const { t, i18n } = useTranslation();
  const people = useUsers().data ?? [];
  const base = `/api/apps/${encodeURIComponent(props.detail.id)}`;
  const versions = useAppResource<Version[]>(props, `${base}/records/versions`, []);
  const audit = useAppResource<AuditEntry[]>(props, `${base}/records/audit`, []);
  return (
    <>
      <SettingsSection
        title={t("appConsole.history.versions")}
        description={t("appConsole.history.versionsHelp")}
      >
        <LoadState loading={versions.loading} failed={versions.failed} onRetry={versions.reload} />
        <ul className="divide-y rounded-lg border empty:hidden">
          {versions.value.map((version, index) => {
            const changes = revisionChanges(versions.value[index + 1]?.manifest, version.manifest);
            return (
              <li key={version.revision} className="space-y-0.5 p-3 text-xs">
                <span className="block font-medium">
                  {t("appConsole.revision", { revision: version.revision })}
                  {props.detail.installation?.revision === version.revision
                    ? ` · ${t("appConsole.history.approved")}`
                    : ""}
                </span>
                <span className="block text-muted-foreground">
                  {formatTime(version.created_at, i18n.language)}
                </span>
                {changes.length ? (
                  <span className="block break-all font-mono text-muted-foreground">
                    {changes.join(", ")}
                  </span>
                ) : null}
              </li>
            );
          })}
        </ul>
      </SettingsSection>
      <SettingsSection title={t("appConsole.history.audit")}>
        <LoadState loading={audit.loading} failed={audit.failed} onRetry={audit.reload} />
        <ul className="divide-y rounded-lg border empty:hidden">
          {audit.value.map((entry) => (
            <li key={entry.id} className="flex flex-wrap gap-x-2 p-3 text-xs">
              <span className="font-mono">{entry.action}</span>
              <span className="text-muted-foreground">
                {people.find((user) => user.id === entry.actor_id)?.display_name ?? entry.actor_id}
              </span>
              <span className="text-muted-foreground">
                {formatTime(entry.created_at, i18n.language)}
              </span>
            </li>
          ))}
        </ul>
      </SettingsSection>
    </>
  );
}

type AppWebhook = ChannelWebhook & { channel_id: string; channel_name: string };

export function WebhooksSection(props: SectionProps) {
  const { t } = useTranslation();
  const { currentUserId, isAdmin } = useCurrentUser();
  const { detail, client } = props;
  const list = useAppResource<{ webhooks: AppWebhook[]; hidden_count: number }>(
    props,
    `/api/apps/${encodeURIComponent(detail.id)}/webhooks`,
    { webhooks: [], hidden_count: 0 },
  );
  // Matches the server's channel-manager rule; the server still checks it on every request.
  const channels = (useActiveChannels().data ?? []).filter(
    (channel) =>
      !channel.is_dm &&
      !channel.archive_at &&
      (isAdmin || channel.created_user_id === currentUserId),
  );
  const [name, setName] = useState("Webhook");
  const [channelId, setChannelId] = useState("");
  const [confirmation, setConfirmation] = useState<{
    id: string;
    action: "delete" | "rotate";
  } | null>(null);
  const { busy, error, run } = useAction();
  const installed = detail.status === "installed";
  return (
    <>
      <SettingsSection
        title={t("appConsole.webhooks.create")}
        description={t("appConsole.webhooks.createHelp")}
      >
        {installed ? (
          <form
            autoComplete="off"
            className="flex flex-wrap items-end gap-2"
            onSubmit={(event) => {
              event.preventDefault();
              if (!name.trim() || !channelId) return;
              void run(async () => {
                await client.post(`/api/channels/${encodeURIComponent(channelId)}/webhooks`, {
                  name: name.trim(),
                  app_id: detail.id,
                });
                setName("Webhook");
                list.reload();
              });
            }}
          >
            <label className="flex min-w-40 flex-1 flex-col gap-1 text-sm">
              {t("webhooks.name")}
              <Input
                className={inputClassName()}
                value={name}
                maxLength={80}
                onChange={(event) => setName(event.target.value)}
              />
            </label>
            <label className="flex min-w-40 flex-1 flex-col gap-1 text-sm">
              {t("appConsole.webhooks.channel")}
              <Select value={channelId} onChange={(event) => setChannelId(event.target.value)}>
                <option value="">{t("apps.ui.chooseChannel")}</option>
                {channels.map((channel) => (
                  <option key={channel.id} value={channel.id}>
                    #{channel.channel_name}
                  </option>
                ))}
              </Select>
            </label>
            <Button type="submit" disabled={busy || !name.trim() || !channelId}>
              {t("webhooks.create")}
            </Button>
          </form>
        ) : (
          <p className="text-sm text-muted-foreground">
            {t("appConsole.credentials.needsInstall")}
          </p>
        )}
        {installed && channels.length === 0 ? (
          <p className="text-xs text-muted-foreground">{t("appConsole.webhooks.noChannels")}</p>
        ) : null}
      </SettingsSection>
      <SettingsSection title={t("appConsole.webhooks.list")}>
        <LoadState loading={list.loading} failed={list.failed} onRetry={list.reload} />
        <ErrorText error={error} />
        {!list.loading && !list.failed && list.value.webhooks.length === 0 ? (
          <p className="text-sm text-muted-foreground">{t("webhooks.empty")}</p>
        ) : null}
        <ul className="flex flex-col gap-3">
          {list.value.webhooks.map((hook) => (
            <WebhookItem
              key={hook.id}
              hook={hook}
              basePath={`/api/channels/${encodeURIComponent(hook.channel_id)}/webhooks`}
              busy={busy}
              run={(action) =>
                run(async () => {
                  await action();
                  setConfirmation(null);
                })
              }
              confirmation={confirmation?.id === hook.id ? confirmation.action : null}
              onConfirm={(action) => setConfirmation(action ? { id: hook.id, action } : null)}
              onReplaced={(next) =>
                list.setValue((current) => ({
                  ...current,
                  webhooks: current.webhooks.map((item) =>
                    item.id === hook.id ? { ...hook, ...next } : item,
                  ),
                }))
              }
              onRemoved={() => {
                list.setValue((current) => ({
                  ...current,
                  webhooks: current.webhooks.filter((item) => item.id !== hook.id),
                }));
                // Removing the last webhook disables the app on the server, so reload its status.
                props.refresh();
              }}
            >
              <p className="text-xs text-muted-foreground">#{hook.channel_name}</p>
            </WebhookItem>
          ))}
        </ul>
        {list.value.hidden_count > 0 ? (
          <p className="text-xs text-muted-foreground">
            {t("appConsole.webhooks.hidden", { count: list.value.hidden_count })}
          </p>
        ) : null}
      </SettingsSection>
    </>
  );
}
