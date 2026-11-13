import { useEffect, useMemo, useRef, useState } from "react";
import { LinkIcon, UnlinkIcon } from "lucide-react";
import { useNavigate, useRouterState } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { ApiError, createConfiguredApiClient } from "../../lib/api/client";
import {
  completeSlackLink,
  getSlackLink,
  readSlackLinkResult,
  slackLinkErrorReason,
  startSlackLink,
  unlinkSlack,
  type SlackLinkResult,
  type SlackLinkState,
} from "../../lib/api/slackLink";
import {
  getPrivateSlackChannels,
  hasActivePrivateImport,
  needsRelink,
  startPrivateSlackImport,
  type PrivateSlackChannel,
  type PrivateSlackChannels,
} from "../../lib/api/slackPrivateImport";
import { useCollections } from "../../lib/electric/collections-context";
import { Button } from "../ui/button";
import { Input } from "../ui/input";
import { ConfirmActionDialog } from "../ui/confirm-action-dialog";
import { SettingsSection } from "../ui/control-panel";
import { Spinner } from "../ui/spinner";
import { describeImportError } from "./SlackImportPanel";

const LINK_ERRORS = {
  denied: "slackLink.errors.denied",
  invalid_state: "slackLink.errors.invalid_state",
  team_mismatch: "slackLink.errors.team_mismatch",
  already_linked: "slackLink.errors.already_linked",
  linked_other_account: "slackLink.errors.linked_other_account",
  failed: "slackLink.errors.failed",
} as const;

export function SlackLinkPanel({ navigateTo = (url: string) => window.location.assign(url) }) {
  const { t } = useTranslation();
  const { config } = useCollections();
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
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
        // 連携の完了・解除で状態が決まったあとなら、古い読み込みの失敗は表示しない
        if (!controller.signal.aborted && revision.current === started) {
          setError(t("slackLink.loadError"));
        }
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
        fail(
          cause instanceof ApiError
            ? slackLinkErrorReason(cause.status, cause.body)
            : slackLinkErrorReason(null),
        );
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
            <div className="min-w-0 space-y-1">
              <p className="text-sm break-all">
                {t("slackLink.linkedAs", {
                  name: state.link.slack_user_name || state.link.slack_user_id,
                  team: state.link.team_name || state.link.team_id,
                })}
              </p>
              {state.link.team_allowed === false ? (
                <p className="text-sm text-destructive">{t("slackLink.teamNotAllowed")}</p>
              ) : null}
            </div>
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
      {state?.link && state.link.team_allowed !== false ? (
        <OwnImportSection client={client} />
      ) : null}
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

// Slack の API を読む一覧なので、間隔は長めにする
const POLL_INTERVAL_MS = 5_000;
const POLL_MAX_INTERVAL_MS = 60_000;

/** 連携した Slack アカウントで作った非公開チャンネルを、Slack の API から取り込む */
function OwnImportSection({ client }: { client: ReturnType<typeof createConfiguredApiClient> }) {
  const { t } = useTranslation();
  const [data, setData] = useState<PrivateSlackChannels | null>(null);
  const [error, setError] = useState("");
  // 一覧を取れなかったときのエラー。次に取れたら消える (取り込みの操作のエラーとは分ける)
  const [loadError, setLoadError] = useState("");
  const [pendingId, setPendingId] = useState("");
  const [revision, setRevision] = useState(0);
  // 取り込み先の名前の編集。まだ編集していないチャンネルは、サーバーの提案を使う
  const [names, setNames] = useState<Record<string, string>>({});
  const active = !!data && hasActivePrivateImport(data.channels);

  useEffect(() => {
    const controller = new AbortController();
    let timer: number | undefined;
    let inflight: Promise<void> | null = null;
    let delay = POLL_INTERVAL_MS;
    let lastKey = "";
    // 取得に失敗している間は、進行中の取り込みがなくても、間隔を延ばしながら再試行する
    let failing = false;
    // 前の取得が終わってから次を予約する。取得中の要求は、タイマーでも復帰でも共有する (重ならない)。
    // 状態が変わらない間・失敗した間は、上限付きで間隔を延ばし、変わったら戻す。非表示の間は止めて、戻ったら取り直す。
    // 一覧は Slack の API を呼ぶので、頻度を抑える (`docs/request-design.md`)
    const fetchOnce = (): Promise<void> =>
      getPrivateSlackChannels(client, controller.signal)
        .then((value) => {
          if (controller.signal.aborted) return;
          setData(value);
          setLoadError("");
          failing = false;
          const key = value.channels.map((c) => `${c.slack_channel_id}:${c.status}`).join(",");
          delay = key === lastKey ? Math.min(delay * 2, POLL_MAX_INTERVAL_MS) : POLL_INTERVAL_MS;
          lastKey = key;
        })
        .catch((cause) => {
          if (controller.signal.aborted) return;
          setLoadError(describeImportError(cause, t, false));
          failing = true;
          delay = Math.min(delay * 2, POLL_MAX_INTERVAL_MS);
        });
    const load = (): Promise<void> => {
      inflight ??= fetchOnce().finally(() => {
        inflight = null;
      });
      return inflight;
    };
    const schedule = () => {
      window.clearTimeout(timer);
      if (
        (!active && !failing) ||
        controller.signal.aborted ||
        document.hidden ||
        !navigator.onLine
      )
        return;
      timer = window.setTimeout(() => void load().then(schedule), delay);
    };
    const onVisible = () => {
      if (document.hidden) {
        window.clearTimeout(timer);
        return;
      }
      delay = POLL_INTERVAL_MS;
      void load().then(schedule);
    };
    void load().then(schedule);
    document.addEventListener("visibilitychange", onVisible);
    // オフラインの間は止め、戻ったら取り直す
    window.addEventListener("online", onVisible);
    return () => {
      controller.abort();
      window.clearTimeout(timer);
      document.removeEventListener("visibilitychange", onVisible);
      window.removeEventListener("online", onVisible);
    };
  }, [client, revision, active, t]);

  async function start(channel: PrivateSlackChannel) {
    if (pendingId) return;
    const name = (names[channel.slack_channel_id] ?? channel.suggested_name ?? channel.name).trim();
    setPendingId(channel.slack_channel_id);
    setError("");
    try {
      await startPrivateSlackImport(client, { slack_channel_id: channel.slack_channel_id, name });
    } catch (cause) {
      setError(describeImportError(cause, t, false));
    } finally {
      setPendingId("");
      setRevision((value) => value + 1);
    }
  }

  const unavailable = data && data.channels.length === 0 && data.reason;
  return (
    <SettingsSection
      title={t("slackLink.privateTitle")}
      description={t("slackLink.privateDescription")}
    >
      {data === null && !error && !loadError ? (
        <Spinner label={t("slackLink.privateLoading")} className="justify-start" />
      ) : null}
      {unavailable ? (
        <p className="text-sm text-muted-foreground">
          {needsRelink(data.reason)
            ? t("slackLink.privateRelink")
            : t("slackLink.privateUnavailable")}
        </p>
      ) : null}
      {data && !data.reason && data.channels.length === 0 ? (
        <p className="text-sm text-muted-foreground">{t("slackLink.privateEmpty")}</p>
      ) : null}
      {data && data.channels.length > 0 ? (
        <ul className="flex flex-col gap-3">
          {data.channels.map((channel) => {
            const id = channel.slack_channel_id;
            const label = `#${channel.name}`;
            return (
              <li
                key={id}
                className="flex flex-col gap-2 rounded-md border border-border p-3 sm:flex-row sm:items-center"
              >
                <div className="min-w-0 flex-1">
                  <p className="truncate text-sm font-medium">{label}</p>
                  <p className="text-xs text-muted-foreground">
                    {t("slackLink.privateMembers", { count: channel.members })}
                    {channel.is_archived ? ` · ${t("slackLink.privateArchived")}` : ""}
                  </p>
                  {channel.status !== "new" ? (
                    <p className="text-xs text-muted-foreground" role="status">
                      {channel.status === "hidden"
                        ? t("slackLink.privateStatus.hidden")
                        : t(`slackLink.privateStatus.${channel.status}`, {
                            name: channel.channel_name ?? "",
                          })}
                    </p>
                  ) : null}
                </div>
                {channel.status === "new" ? (
                  <>
                    <Input
                      aria-label={t("slackLink.privateName", { name: label })}
                      value={names[id] ?? channel.suggested_name ?? channel.name}
                      onChange={(event) =>
                        setNames((current) => ({ ...current, [id]: event.target.value }))
                      }
                      className="sm:w-56"
                    />
                    <Button
                      type="button"
                      disabled={!!pendingId}
                      aria-label={t("slackLink.privateStart", { name: label })}
                      onClick={() => void start(channel)}
                    >
                      {pendingId === id
                        ? t("slackLink.privateStarting")
                        : t("slackLink.privateStartButton")}
                    </Button>
                  </>
                ) : null}
              </li>
            );
          })}
        </ul>
      ) : null}
      {loadError ? (
        <p role="alert" className="text-sm text-destructive">
          {loadError}
        </p>
      ) : null}
      {error ? (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      ) : null}
    </SettingsSection>
  );
}
