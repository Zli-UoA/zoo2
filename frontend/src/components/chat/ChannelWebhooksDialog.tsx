import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { CopyIcon, EyeIcon, EyeOffIcon } from "lucide-react";
import { toast } from "sonner";
import { createConfiguredApiClient } from "../../lib/api/client";
import { getCollections } from "../../lib/electric/collections";
import { joinUrl } from "../../lib/config";
import type { ChannelRow } from "../../lib/electric/types";
import { describeApiError } from "../../hooks/useChannelAdmin";
import { Button } from "../ui/button";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "../ui/dialog";
import { inputClassName } from "../ui/input-styles";

export interface ChannelWebhook {
  id: string;
  name: string;
  path: string;
  secret: string;
  created_at: string;
}

export function webhookUrl(baseUrl: string, path: string, origin: string) {
  return new URL(joinUrl(baseUrl, path), origin).href;
}

function Credential({
  label,
  value,
  secret = false,
}: {
  label: string;
  value: string;
  secret?: boolean;
}) {
  const { t } = useTranslation();
  const [visible, setVisible] = useState(false);
  async function copy() {
    try {
      await navigator.clipboard.writeText(value);
      toast.success(t("webhooks.copied"));
    } catch {
      toast.error(t("webhooks.copyError"));
    }
  }
  return (
    <label className="flex flex-col gap-1 text-xs">
      {label}
      <span className="flex items-center gap-1">
        <input
          className={`${inputClassName()} min-w-0 flex-1 font-mono text-xs`}
          readOnly
          value={value}
          type={secret && !visible ? "password" : "text"}
          autoComplete="off"
          spellCheck={false}
          onFocus={(e) => e.currentTarget.select()}
        />
        {secret ? (
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            aria-label={visible ? t("webhooks.hideSecret") : t("webhooks.showSecret")}
            onClick={() => setVisible(!visible)}
          >
            {visible ? <EyeOffIcon /> : <EyeIcon />}
          </Button>
        ) : null}
        <Button
          type="button"
          variant="outline"
          size="icon-sm"
          aria-label={t("webhooks.copyLabel", { label })}
          onClick={() => void copy()}
        >
          <CopyIcon />
        </Button>
      </span>
    </label>
  );
}

export function ChannelWebhooksDialog({
  channel,
  onOpenChange,
}: {
  channel: ChannelRow;
  onOpenChange: (open: boolean) => void;
}) {
  const { t } = useTranslation();
  const config = useMemo(() => getCollections().config, []);
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  const path = `/api/channels/${channel.id}/webhooks`;
  const [hooks, setHooks] = useState<ChannelWebhook[]>([]);
  const [loading, setLoading] = useState(true);
  const [reload, setReload] = useState(0);
  const [name, setName] = useState("GitHub");
  const [busy, setBusy] = useState(false);
  const pending = useRef(false);
  const [error, setError] = useState<string | null>(null);
  const [confirmation, setConfirmation] = useState<{
    hook: ChannelWebhook;
    action: "delete" | "rotate";
  } | null>(null);

  useEffect(() => {
    const controller = new AbortController();
    setLoading(true);
    setError(null);
    void client
      .requestWithTx<ChannelWebhook[]>(path, { signal: controller.signal })
      .then((result) => {
        if (!controller.signal.aborted) setHooks(result.data);
      })
      .catch((cause) => {
        if (!controller.signal.aborted) setError(describeApiError(cause, t));
      })
      .finally(() => {
        if (!controller.signal.aborted) setLoading(false);
      });
    return () => controller.abort();
  }, [client, path, reload, t]);

  async function run(action: () => Promise<void>) {
    if (pending.current) return;
    pending.current = true;
    setBusy(true);
    setError(null);
    try {
      await action();
      setConfirmation(null);
    } catch (cause) {
      setError(describeApiError(cause, t));
    } finally {
      pending.current = false;
      setBusy(false);
    }
  }

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!pending.current) onOpenChange(open);
      }}
    >
      <DialogContent className="max-h-[85dvh] overflow-y-auto sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>{t("webhooks.title")}</DialogTitle>
          <DialogDescription>
            {t("webhooks.description", { channel: channel.channel_name })}
          </DialogDescription>
        </DialogHeader>
        <p className="text-xs text-muted-foreground">{t("webhooks.instructions")}</p>
        {error ? (
          <div role="alert" className="text-sm text-destructive">
            {error}{" "}
            <Button
              variant="ghost"
              size="sm"
              disabled={busy || loading}
              onClick={() => setReload((n) => n + 1)}
            >
              {t("webhooks.retry")}
            </Button>
          </div>
        ) : null}
        {loading ? (
          <p role="status">{t("webhooks.loading")}</p>
        ) : (
          <>
            <form
              className="flex items-end gap-2"
              onSubmit={(event) => {
                event.preventDefault();
                if (!name.trim()) return;
                void run(async () => {
                  const result = await client.post<ChannelWebhook>(path, { name: name.trim() });
                  setHooks((current) => [...current, result.data]);
                  setName("GitHub");
                });
              }}
            >
              <label className="flex min-w-0 flex-1 flex-col gap-1 text-sm">
                {t("webhooks.name")}
                <input
                  className={inputClassName()}
                  value={name}
                  maxLength={80}
                  disabled={busy || channel.archive_at !== null}
                  onChange={(e) => setName(e.target.value)}
                />
              </label>
              <Button type="submit" disabled={busy || !name.trim() || channel.archive_at !== null}>
                {t("webhooks.create")}
              </Button>
            </form>
            {channel.archive_at !== null ? (
              <p className="text-xs text-muted-foreground">{t("webhooks.archived")}</p>
            ) : null}
            {hooks.length === 0 ? (
              <p className="py-3 text-sm text-muted-foreground">{t("webhooks.empty")}</p>
            ) : null}
            <ul className="flex flex-col gap-3">
              {hooks.map((hook) => (
                <li key={hook.id} className="flex flex-col gap-3 rounded-lg border p-3">
                  <h3 className="break-words text-sm font-semibold">{hook.name}</h3>
                  <Credential
                    label="URL"
                    value={webhookUrl(config.apiBaseUrl, hook.path, window.location.origin)}
                  />
                  <Credential key={hook.secret} label="Secret" value={hook.secret} secret />
                  <div className="flex justify-end gap-2">
                    <Button
                      variant="outline"
                      size="sm"
                      disabled={busy}
                      onClick={() => setConfirmation({ hook, action: "rotate" })}
                    >
                      {t("webhooks.rotate")}
                    </Button>
                    <Button
                      variant="destructive"
                      size="sm"
                      disabled={busy}
                      onClick={() => setConfirmation({ hook, action: "delete" })}
                    >
                      {t("webhooks.delete")}
                    </Button>
                  </div>
                  {confirmation?.hook.id === hook.id ? (
                    <div className="flex flex-col gap-2 rounded border p-2 text-sm">
                      <p>
                        {confirmation.action === "delete"
                          ? t("webhooks.deleteConfirm")
                          : t("webhooks.rotateConfirm")}
                      </p>
                      <div className="flex justify-end gap-2">
                        <Button
                          variant="ghost"
                          size="sm"
                          disabled={busy}
                          onClick={() => setConfirmation(null)}
                        >
                          {t("common.cancelShort")}
                        </Button>
                        <Button
                          variant="destructive"
                          size="sm"
                          disabled={busy}
                          onClick={() =>
                            void run(async () => {
                              if (confirmation.action === "delete") {
                                await client.delete(`${path}/${hook.id}`);
                                setHooks((current) =>
                                  current.filter((item) => item.id !== hook.id),
                                );
                              } else {
                                const result = await client.post<ChannelWebhook>(
                                  `${path}/${hook.id}/rotate-secret`,
                                );
                                setHooks((current) =>
                                  current.map((item) => (item.id === hook.id ? result.data : item)),
                                );
                              }
                            })
                          }
                        >
                          {t("webhooks.confirm")}
                        </Button>
                      </div>
                    </div>
                  ) : null}
                </li>
              ))}
            </ul>
          </>
        )}
      </DialogContent>
    </Dialog>
  );
}
