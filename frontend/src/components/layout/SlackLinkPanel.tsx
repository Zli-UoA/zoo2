import { useEffect, useMemo, useRef, useState } from "react";
import { CopyIcon, LinkIcon, UnlinkIcon } from "lucide-react";
import { useNavigate, useRouterState } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { useCurrentUser } from "../../hooks/useCurrentUser";
import { ApiError, createConfiguredApiClient } from "../../lib/api/client";
import {
  completeSlackLink,
  getSlackLink,
  getSlackManifest,
  readSlackLinkResult,
  slackLinkErrorReason,
  startSlackLink,
  unlinkSlack,
  type SlackLinkResult,
  type SlackLinkState,
} from "../../lib/api/slackLink";
import {
  SLACK_IMPORT_RUNNING,
  getOwnSlackImports,
  startOwnSlackImport,
  type OwnSlackImports,
} from "../../lib/api/slackImport";
import { useCollections } from "../../lib/electric/collections-context";
import { Button } from "../ui/button";
import { ConfirmActionDialog } from "../ui/confirm-action-dialog";
import { SettingsSection } from "../ui/control-panel";
import { Spinner } from "../ui/spinner";
import { Textarea } from "../ui/textarea";
import { MappingForm, SlackImportRunStatus, describeImportError } from "./SlackImportPanel";

const LINK_ERRORS = {
  denied: "slackLink.errors.denied",
  invalid_state: "slackLink.errors.invalid_state",
  team_mismatch: "slackLink.errors.team_mismatch",
  already_linked: "slackLink.errors.already_linked",
  failed: "slackLink.errors.failed",
} as const;

export function SlackLinkPanel({ navigateTo = (url: string) => window.location.assign(url) }) {
  const { t } = useTranslation();
  const { config } = useCollections();
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  const { isAdmin } = useCurrentUser();
  const navigate = useNavigate();
  const search = useRouterState({
    select: (state) => state.location.search as Record<string, unknown>,
  });
  const [state, setState] = useState<SlackLinkState | null>(null);
  const [error, setError] = useState("");
  const [pending, setPending] = useState(false);
  const [confirmUnlink, setConfirmUnlink] = useState(false);
  // 連携の完了・解除で状態を書き換えるたびに進める。それより前に始めた読み込みの結果は捨てる。
  const revision = useRef(0);

  useEffect(() => {
    const controller = new AbortController();
    const started = revision.current;
    getSlackLink(client, controller.signal)
      .then((value) => {
        if (!controller.signal.aborted && revision.current === started) setState(value);
      })
      .catch(() => {
        if (!controller.signal.aborted) setError(t("slackLink.loadError"));
      });
    return () => controller.abort();
  }, [client, t]);

  // Slack から戻ってきたら連携を完了し、URL から code / state を消す
  const result = readSlackLinkResult(search);
  const handled = useRef<SlackLinkResult | null>(null);
  useEffect(() => {
    // 同じ code を二重に送らない (StrictMode の再実行や URL の書き換え待ちの間も)
    if (!result || JSON.stringify(result) === JSON.stringify(handled.current)) return;
    handled.current = result;
    void navigate({ to: "/settings", search: { tab: "slack" }, replace: true });
    const fail = (reason: string) => {
      // toast は消えるので、理由が分かるようパネル内に残す
      setError(t(LINK_ERRORS[reason as keyof typeof LINK_ERRORS] ?? LINK_ERRORS.failed));
    };
    if (result.status === "error") {
      fail(result.reason);
      return;
    }
    setPending(true);
    revision.current += 1;
    completeSlackLink(client, result.code, result.state)
      .then((value) => {
        setState(value);
        toast.success(t("slackLink.linked"));
      })
      .catch((cause) => {
        fail(slackLinkErrorReason(cause instanceof ApiError ? cause.status : null));
        // 初期の読み込みは捨てているので、連携をやり直せるよう状態を読み直す
        const started = ++revision.current;
        getSlackLink(client)
          .then((value) => {
            if (revision.current === started) setState(value);
          })
          .catch(() => setError(t("slackLink.loadError")));
      })
      .finally(() => setPending(false));
  }, [result, client, navigate, t]);

  async function link() {
    if (pending) return;
    setPending(true);
    setError("");
    try {
      navigateTo((await startSlackLink(client)).authorize_url);
    } catch {
      setError(t("slackLink.errors.failed"));
      setPending(false);
    }
  }

  async function unlink() {
    if (pending) return;
    setPending(true);
    setError("");
    try {
      const next = await unlinkSlack(client);
      revision.current += 1;
      setState(next);
      setConfirmUnlink(false);
      toast.success(t("slackLink.unlinked"));
    } catch {
      setError(t("slackLink.unlinkFailed"));
    } finally {
      setPending(false);
    }
  }

  return (
    <div className="space-y-4">
      <SettingsSection title={t("slackLink.title")} description={t("slackLink.description")}>
        {state === null && !error ? (
          <Spinner label={t("slackLink.loading")} className="justify-start" />
        ) : null}
        {state && !state.configured ? (
          <p className="text-sm text-muted-foreground">{t("slackLink.notConfigured")}</p>
        ) : null}
        {state?.configured && state.link ? (
          <div className="flex flex-wrap items-center justify-between gap-3">
            <p className="min-w-0 text-sm break-all">
              {t("slackLink.linkedAs", {
                name: state.link.slack_user_name || state.link.slack_user_id,
                team: state.link.team_name || state.link.team_id,
              })}
            </p>
            <Button
              variant="outline"
              disabled={pending}
              onClick={() => {
                setError("");
                setConfirmUnlink(true);
              }}
            >
              <UnlinkIcon />
              {t("slackLink.unlink")}
            </Button>
          </div>
        ) : null}
        {state?.configured && !state.link ? (
          <Button disabled={pending} onClick={() => void link()}>
            <LinkIcon />
            {t("slackLink.link")}
          </Button>
        ) : null}
        {error ? (
          <p role="alert" className="text-sm text-destructive">
            {error}
          </p>
        ) : null}
      </SettingsSection>
      {state?.link ? <OwnImportSection client={client} /> : null}
      {isAdmin ? <SlackManifestSection client={client} /> : null}
      <ConfirmActionDialog
        open={confirmUnlink}
        onOpenChange={setConfirmUnlink}
        title={t("slackLink.unlinkTitle")}
        description={t("slackLink.unlinkDescription")}
        confirmLabel={t("slackLink.unlink")}
        pending={pending}
        // 背後のパネルはモーダルの裏になるので、解除できなかった理由はダイアログにも出す
        error={error === t("slackLink.unlinkFailed") ? error : undefined}
        onConfirm={unlink}
      />
    </div>
  );
}

const POLL_INTERVAL_MS = 2_000;

/** 連携した Slack アカウントで作ったチャンネルを、管理者が保存した ZIP から取り込む */
function OwnImportSection({ client }: { client: ReturnType<typeof createConfiguredApiClient> }) {
  const { t } = useTranslation();
  const [data, setData] = useState<OwnSlackImports | null>(null);
  const [error, setError] = useState("");
  const [pending, setPending] = useState(false);
  const [revision, setRevision] = useState(0);
  // 取り込みを始めたら選択を戻し、続けて別のチャンネルを選べるようにする
  const [form, setForm] = useState(0);
  const latest = data?.imports[0];
  const running = !!latest && SLACK_IMPORT_RUNNING.includes(latest.status);

  useEffect(() => {
    const controller = new AbortController();
    const load = () =>
      getOwnSlackImports(client, controller.signal)
        .then((value) => {
          if (!controller.signal.aborted) setData(value);
        })
        .catch((cause) => {
          if (!controller.signal.aborted) setError(describeImportError(cause, t, false));
        });
    void load();
    const timer = running ? window.setInterval(load, POLL_INTERVAL_MS) : undefined;
    return () => {
      controller.abort();
      window.clearInterval(timer);
    };
  }, [client, revision, running, t]);

  return (
    <SettingsSection
      title={t("slackLink.importTitle")}
      description={t("slackLink.importDescription")}
    >
      {data === null && !error ? (
        <Spinner label={t("slackLink.importLoading")} className="justify-start" />
      ) : null}
      {data && !data.export ? (
        <p className="text-sm text-muted-foreground">{t("slackLink.importNoExport")}</p>
      ) : null}
      {data?.export && data.channels.length === 0 ? (
        <p className="text-sm text-muted-foreground">{t("slackLink.importNoChannels")}</p>
      ) : null}
      {data?.export && data.channels.length > 0 ? (
        <MappingForm
          key={form}
          channels={data.channels}
          summary={t("slackLink.importSummary", {
            name: data.export.file_name,
            count: data.channels.length,
          })}
          pending={pending || running}
          targetsFor={(channel) =>
            (
              data.channels.find((own) => own.slack_channel_id === channel.slack_channel_id)
                ?.targets ?? []
            ).map((target) => ({
              id: target.id,
              channel_name: target.name,
              is_private: target.is_private,
            }))
          }
          onSubmit={async (mappings) => {
            if (pending) return;
            setPending(true);
            setError("");
            try {
              const started = await startOwnSlackImport(client, mappings);
              setData((current) =>
                current ? { ...current, imports: [started, ...current.imports] } : current,
              );
              setForm((value) => value + 1);
            } catch (cause) {
              setError(describeImportError(cause, t, false));
            } finally {
              setPending(false);
              setRevision((value) => value + 1);
            }
          }}
        />
      ) : null}
      {latest ? <SlackImportRunStatus job={latest} /> : null}
      {error ? (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      ) : null}
    </SettingsSection>
  );
}

function SlackManifestSection({
  client,
}: {
  client: ReturnType<typeof createConfiguredApiClient>;
}) {
  const { t } = useTranslation();
  const [manifest, setManifest] = useState("");
  const [configured, setConfigured] = useState(true);
  const [error, setError] = useState("");
  useEffect(() => {
    const controller = new AbortController();
    getSlackManifest(client, controller.signal)
      .then((value) => {
        if (controller.signal.aborted) return;
        setManifest(JSON.stringify(value.manifest, null, 2));
        setConfigured(value.configured);
      })
      .catch(() => {
        if (!controller.signal.aborted) setError(t("slackLink.manifestError"));
      });
    return () => controller.abort();
  }, [client, t]);

  async function copy() {
    try {
      await navigator.clipboard.writeText(manifest);
      toast.success(t("slackLink.manifestCopied"));
    } catch {
      toast.error(t("slackLink.manifestCopyFailed"));
    }
  }

  return (
    <SettingsSection
      title={t("slackLink.manifestTitle")}
      description={t("slackLink.manifestDescription")}
    >
      {!configured ? (
        <p className="text-sm text-muted-foreground">{t("slackLink.manifestNotConfigured")}</p>
      ) : null}
      {manifest ? (
        <>
          <Textarea
            readOnly
            aria-label={t("slackLink.manifestTitle")}
            className="h-48 font-mono text-xs"
            value={manifest}
          />
          <Button variant="outline" onClick={() => void copy()}>
            <CopyIcon />
            {t("slackLink.copyManifest")}
          </Button>
        </>
      ) : null}
      {error ? (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      ) : null}
    </SettingsSection>
  );
}
