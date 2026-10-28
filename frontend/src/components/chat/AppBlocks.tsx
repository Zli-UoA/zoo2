import { MessageCard, parseMessageCard } from "./messages/MessageCard";
import { openApp } from "./AppIdentity";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { createConfiguredApiClient } from "../../lib/api/client";
import { useCollections } from "../../lib/electric/collections-context";
import { useShell } from "../layout/shell-context";
import { Button } from "../ui/button";
import { inputClassName } from "../ui/input-styles";
import type { FeedMessage } from "./messages/messageFormat";

type Block =
  | { type: "text"; text: string }
  | { type: "button"; action_id: string; label: string; value?: string }
  | { type: "input"; action_id: string; label: string; required?: boolean }
  | { type: "select"; action_id: string; label: string; options: string[] };
export function parseBlocks(value: unknown): Block[] {
  if (!Array.isArray(value) || value.length > 50) return [];
  return value.filter((b): b is Block => {
    if (!b || typeof b !== "object") return false;
    if (b.type === "text") return typeof b.text === "string";
    if (
      !["button", "input", "select"].includes(b.type) ||
      typeof b.action_id !== "string" ||
      typeof b.label !== "string"
    )
      return false;
    return (
      b.type !== "select" ||
      (Array.isArray(b.options) && b.options.every((s: unknown) => typeof s === "string"))
    );
  });
}
export function AppBlocks({
  blocks,
  appId,
  surfaceId,
  messageId,
  channelId,
  submit = false,
  onDone,
}: {
  blocks: unknown;
  appId: string;
  surfaceId?: string;
  messageId?: string;
  channelId?: string | null;
  submit?: boolean;
  onDone?: () => void;
}) {
  const { t } = useTranslation();
  const { config } = useCollections();
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  const [values, setValues] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  async function invoke(kind: string, actionId?: string, value?: string) {
    if (busy) return;
    setBusy(true);
    setError("");
    try {
      await client.request(`/api/apps/${appId}/invoke`, {
        method: "POST",
        body: {
          kind,
          surface_id: surfaceId,
          message_id: messageId,
          channel_id: channelId,
          action_id: actionId,
          values: {
            ...values,
            ...(actionId ? { [actionId]: value ?? values[actionId] ?? "" } : {}),
          },
        },
      });
      if (kind === "submit") onDone?.();
    } catch {
      setError(t("apps.actionError"));
    } finally {
      setBusy(false);
    }
  }
  return (
    <form
      className="my-2 space-y-3"
      onSubmit={(e) => {
        e.preventDefault();
        void invoke("submit");
      }}
    >
      {parseBlocks(blocks).map((b, index) => {
        if (b.type === "text")
          return (
            <p key={index} className="whitespace-pre-wrap break-words text-sm">
              {b.text}
            </p>
          );
        if (b.type === "button")
          return (
            <Button
              key={b.action_id}
              type="button"
              size="sm"
              variant="outline"
              disabled={busy}
              onClick={() => void invoke("action", b.action_id, b.value)}
            >
              {b.label}
            </Button>
          );
        return (
          <label key={b.action_id} className="flex flex-col gap-1 text-sm">
            {b.label}
            {b.type === "input" ? (
              <input
                className={inputClassName()}
                required={b.required}
                maxLength={4000}
                value={values[b.action_id] ?? ""}
                onChange={(e) => setValues({ ...values, [b.action_id]: e.target.value })}
              />
            ) : (
              <select
                className={inputClassName()}
                value={values[b.action_id] ?? ""}
                onChange={(e) => {
                  const value = e.target.value;
                  setValues({ ...values, [b.action_id]: value });
                  if (!submit) void invoke("action", b.action_id, value);
                }}
              >
                <option value="" />
                {b.options.map((option) => (
                  <option key={option}>{option}</option>
                ))}
              </select>
            )}
          </label>
        );
      })}
      {submit ? (
        <Button type="submit" disabled={busy}>
          {t("apps.submit")}
        </Button>
      ) : null}
      {error ? (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      ) : null}
    </form>
  );
}
export function MessageAppContent({ message }: { message: FeedMessage }) {
  if (!message.app_blocks && !message.app_attachments && !message.app_unfurls) return null;
  return <ConnectedMessageAppContent message={message} />;
}
function ConnectedMessageAppContent({ message }: { message: FeedMessage }) {
  const shell = useShell();
  const { config } = useCollections();
  const { t } = useTranslation();
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  const [error, setError] = useState("");
  let unfurls: { app_id: string; name: string; url?: string; card: unknown }[] = [];
  try {
    const value: unknown = JSON.parse(message.app_unfurls ?? "[]");
    if (Array.isArray(value))
      unfurls = value.filter((v) => typeof v?.app_id === "string" && typeof v?.name === "string");
  } catch {
    /* Ignore malformed link metadata. */
  }
  let blocks: unknown = [];
  let files: { id: string; name: string }[] = [];
  try {
    blocks = JSON.parse(message.app_blocks ?? "[]");
  } catch {
    /* Keep text readable. */
  }
  try {
    const parsed: unknown = JSON.parse(message.app_attachments ?? "[]");
    if (Array.isArray(parsed))
      files = parsed.filter((v) => typeof v?.id === "string" && typeof v?.name === "string");
  } catch {
    /* Keep text readable. */
  }
  async function download(id: string) {
    try {
      const result = await client.get<{ url: string }>(`/api/app-files/${id}/download`);
      if (result.url.startsWith("https://"))
        window.open(result.url, "_blank", "noopener,noreferrer");
    } catch {
      setError(t("apps.actionError"));
    }
  }
  return (
    <>
      {(message.app_origin_id || message.app_id) && parseBlocks(blocks).length ? (
        <AppBlocks
          blocks={blocks}
          appId={(message.app_origin_id || message.app_id)!}
          messageId={message.id}
          channelId={shell.selectedChannelId}
        />
      ) : null}
      {unfurls.map((unfurl, index) => {
        const card = parseMessageCard(JSON.stringify(unfurl.card));
        return card ? (
          <div key={index}>
            <button
              className="text-xs text-muted-foreground"
              type="button"
              onClick={() => openApp(unfurl.app_id)}
            >
              {unfurl.name} · APP
            </button>
            <MessageCard card={card} />
            {unfurl.url && shell.selectedChannelId ? (
              <div className="flex gap-2">
                {(["details", "comments"] as const).map((kind) => (
                  <Button
                    key={kind}
                    size="sm"
                    variant="ghost"
                    onClick={() => {
                      void client
                        .request(`/api/apps/${unfurl.app_id}/entities/requests`, {
                          method: "POST",
                          body: {
                            kind,
                            url: unfurl.url,
                            message_id: message.id,
                            channel_id: shell.selectedChannelId,
                          },
                        })
                        .catch(() => setError(t("apps.actionError")));
                    }}
                  >
                    {t(`apps.entity.${kind}`)}
                  </Button>
                ))}
              </div>
            ) : null}
          </div>
        ) : null;
      })}
      {files.map((file) => (
        <Button key={file.id} type="button" variant="link" onClick={() => void download(file.id)}>
          {file.name}
        </Button>
      ))}
      {error ? <p role="alert">{error}</p> : null}
    </>
  );
}
