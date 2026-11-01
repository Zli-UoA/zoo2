import { useEffect, useMemo, useRef, useState } from "react";
import { LockIcon, UploadIcon } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { useActiveChannels } from "../../hooks/useRealtime";
import { ApiError, createConfiguredApiClient } from "../../lib/api/client";
import {
  SLACK_IMPORT_RUNNING,
  cancelSlackImport,
  defaultMappings,
  fetchSlackImportFiles,
  isSlackExport,
  listSlackImports,
  mappingFor,
  splitSlackImports,
  retryableFiles,
  startSlackImport,
  uploadSlackExport,
  type PutWithProgress,
  type SlackChannelMapping,
  type SlackChannelSummary,
  type SlackImport,
} from "../../lib/api/slackImport";
import { useCollections } from "../../lib/electric/collections-context";
import { Button } from "../ui/button";
import { ConfirmActionDialog } from "../ui/confirm-action-dialog";
import { SettingsSection } from "../ui/control-panel";
import { Input } from "../ui/input";
import { Select } from "../ui/select";
import { Spinner } from "../ui/spinner";

const POLL_INTERVAL_MS = 2_000;

export function SlackImportPanel({ put }: { put?: PutWithProgress }) {
  const { t } = useTranslation();
  const { config } = useCollections();
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  const [imports, setImports] = useState<SlackImport[] | null>(null);
  const [revision, setRevision] = useState(0);
  const [error, setError] = useState("");
  const [upload, setUpload] = useState<number | null>(null);
  const fileInput = useRef<HTMLInputElement>(null);

  const running = imports?.some((job) => SLACK_IMPORT_RUNNING.includes(job.status)) ?? false;
  useEffect(() => {
    const controller = new AbortController();
    const load = () =>
      listSlackImports(client, controller.signal)
        .then((value) => {
          if (!controller.signal.aborted) setImports(value);
        })
        .catch((cause) => {
          if (!controller.signal.aborted) setError(describeImportError(cause, t));
        });
    void load();
    const timer = running ? window.setInterval(load, POLL_INTERVAL_MS) : undefined;
    return () => {
      controller.abort();
      window.clearInterval(timer);
    };
  }, [client, revision, running, t]);

  const reload = () => setRevision((value) => value + 1);
  // 失敗の多くは別のタブ・管理者が先に状態を変えたため (409) なので、一覧を取り直して今の状態を見せる
  const fail = (cause: unknown) => {
    setError(describeImportError(cause, t));
    reload();
  };
  const replace = (job: SlackImport) =>
    setImports((current) => [job, ...(current ?? []).filter((item) => item.id !== job.id)]);

  async function onFile(file: File | undefined) {
    if (!file || upload !== null) return;
    setError("");
    setUpload(0);
    try {
      replace(await uploadSlackExport(client, file, setUpload, put));
    } catch (cause) {
      setError(describeImportError(cause, t));
      reload();
    } finally {
      setUpload(null);
      if (fileInput.current) fileInput.current.value = "";
    }
  }

  const { cards, history } = splitSlackImports(imports ?? []);
  // 解析が終わるまでは次の ZIP を受け付けない (解析が終わると古い ZIP を置き換える)
  const busy = cards.some(
    (job) => isSlackExport(job) && (job.status === "uploading" || job.status === "analyzing"),
  );

  return (
    <div className="space-y-4">
      <SettingsSection title={t("slackImport.title")} description={t("slackImport.description")}>
        <input
          ref={fileInput}
          type="file"
          accept=".zip,application/zip"
          className="sr-only"
          aria-label={t("slackImport.chooseFile")}
          onChange={(event) => void onFile(event.target.files?.[0])}
        />
        <Button
          variant="outline"
          disabled={upload !== null || !!busy}
          onClick={() => fileInput.current?.click()}
        >
          <UploadIcon />
          {t("slackImport.chooseFile")}
        </Button>
        {upload !== null ? <Progress label={t("slackImport.uploading")} value={upload} /> : null}
        {error ? (
          <p role="alert" className="text-sm text-destructive">
            {error}
          </p>
        ) : null}
      </SettingsSection>

      {imports === null && !error ? <Spinner label={t("slackImport.loading")} /> : null}
      {cards.map((job) => (
        <ImportCard
          key={job.id}
          job={job}
          onChange={replace}
          onStarted={(started) => {
            replace(started);
            // 以前の取り込み先などの表示を新しくする
            reload();
          }}
          onError={fail}
          client={client}
        />
      ))}
      {history.length > 0 ? (
        <SettingsSection title={t("slackImport.history")}>
          <ul className="divide-y text-sm">
            {history.map((job) => (
              <li key={job.id} className="flex flex-wrap items-center justify-between gap-2 py-2">
                <span className="min-w-0 break-all">{jobTitle(job, t)}</span>
                <span className="text-muted-foreground">
                  {t(`slackImport.status.${job.status}`)} ·{" "}
                  {new Date(job.created_at).toLocaleString()}
                </span>
                {retryableFiles(job) > 0 ? (
                  <FileSummary job={job} client={client} onChange={replace} onError={fail} />
                ) : null}
              </li>
            ))}
          </ul>
        </SettingsSection>
      ) : null}
    </div>
  );
}

function ImportCard({
  job,
  client,
  onChange,
  onStarted,
  onError,
}: {
  job: SlackImport;
  client: ReturnType<typeof createConfiguredApiClient>;
  onChange: (job: SlackImport) => void;
  onStarted: (job: SlackImport) => void;
  onError: (cause: unknown) => void;
}) {
  const { t } = useTranslation();
  const [pending, setPending] = useState(false);
  const [confirmCancel, setConfirmCancel] = useState(false);
  // 取り込みを始めたら選択を戻し、続けて別のチャンネルを選べるようにする
  const [form, setForm] = useState(0);

  async function run(action: () => Promise<SlackImport>) {
    if (pending) return;
    setPending(true);
    try {
      onChange(await action());
    } catch (cause) {
      onError(cause);
    } finally {
      setPending(false);
    }
  }

  const exported = isSlackExport(job);
  const cancellable = exported
    ? ["uploading", "ready", "failed"].includes(job.status)
    : job.status === "failed";
  return (
    <SettingsSection title={jobTitle(job, t)} description={t(`slackImport.status.${job.status}`)}>
      {job.status === "analyzing" || job.status === "uploading" ? (
        <Spinner label={t("slackImport.status.analyzing")} className="justify-start" />
      ) : null}
      {exported && job.status === "ready" && job.analysis ? (
        <MappingForm
          key={form}
          channels={job.analysis.channels}
          userCount={job.analysis.user_count}
          pending={pending}
          onSubmit={async (mappings) => {
            if (pending) return;
            setPending(true);
            try {
              onStarted(await startSlackImport(client, job.id, mappings));
              setForm((value) => value + 1);
            } catch (cause) {
              onError(cause);
            } finally {
              setPending(false);
            }
          }}
        />
      ) : null}
      {job.status === "importing" ? (
        <Progress
          label={t("slackImport.progressMessages", {
            done: job.processed_messages,
            total: job.total_messages,
          })}
          value={job.total_messages > 0 ? job.processed_messages / job.total_messages : 0}
        />
      ) : null}
      {job.status === "fetching_files" ? (
        <Progress
          label={t("slackImport.progressFiles", {
            done: job.processed_files,
            total: job.total_files,
          })}
          value={job.total_files > 0 ? job.processed_files / job.total_files : 0}
        />
      ) : null}
      {job.status === "completed" ? (
        <div className="space-y-2 text-sm">
          <p>{t("slackImport.completed", { count: job.processed_messages })}</p>
          {retryableFiles(job) > 0 ? (
            <FileSummary job={job} client={client} onChange={onChange} onError={onError} />
          ) : null}
        </div>
      ) : null}
      {job.status === "failed" && job.error ? (
        <p role="alert" className="break-words text-sm text-destructive">
          {job.error}
        </p>
      ) : null}
      {cancellable && exported ? (
        <Button variant="ghost" disabled={pending} onClick={() => setConfirmCancel(true)}>
          {t("slackImport.cancel")}
        </Button>
      ) : null}
      {cancellable && !exported ? (
        <Button
          variant="ghost"
          disabled={pending}
          onClick={() => void run(() => cancelSlackImport(client, job.id))}
        >
          {t("slackImport.dismiss")}
        </Button>
      ) : null}
      <ConfirmActionDialog
        open={confirmCancel}
        onOpenChange={setConfirmCancel}
        title={t("slackImport.cancelTitle")}
        description={t("slackImport.cancelDescription")}
        confirmLabel={t("slackImport.cancel")}
        pending={pending}
        onConfirm={async () => {
          await run(() => cancelSlackImport(client, job.id));
          setConfirmCancel(false);
        }}
      />
    </SettingsSection>
  );
}

/** 未取得・取得失敗の添付の件数と、取得し直す操作。履歴の取り込みからも使う。 */
function FileSummary({
  job,
  client,
  onChange,
  onError,
}: {
  job: SlackImport;
  client: ReturnType<typeof createConfiguredApiClient>;
  onChange: (job: SlackImport) => void;
  onError: (cause: unknown) => void;
}) {
  const { t } = useTranslation();
  const [pending, setPending] = useState(false);
  return (
    <div className="w-full space-y-2 text-sm">
      {job.pending_files > 0 ? (
        <p className="text-muted-foreground">
          {t("slackImport.pendingFiles", { count: job.pending_files })}
        </p>
      ) : null}
      {job.failed_files > 0 ? (
        <p className="text-destructive">
          {t("slackImport.failedFiles", { count: job.failed_files })}
        </p>
      ) : null}
      <Button
        variant="outline"
        disabled={pending}
        aria-label={t("slackImport.fetchFilesFor", { name: job.file_name })}
        onClick={() => {
          setPending(true);
          fetchSlackImportFiles(client, job.id)
            .then(onChange, onError)
            .finally(() => setPending(false));
        }}
      >
        {t("slackImport.fetchFiles")}
      </Button>
    </div>
  );
}

function MappingForm({
  channels,
  userCount,
  pending,
  onSubmit,
}: {
  channels: SlackChannelSummary[];
  userCount: number;
  pending: boolean;
  onSubmit: (mappings: SlackChannelMapping[]) => void | Promise<void>;
}) {
  const { t } = useTranslation();
  const zooChannels = useActiveChannels();
  const targets = (zooChannels.data ?? []).filter((channel) => !channel.is_dm);
  const [mappings, setMappings] = useState(() => defaultMappings(channels));
  const update = (index: number, mapping: SlackChannelMapping) =>
    setMappings((current) => current.map((item, i) => (i === index ? mapping : item)));
  const selected = mappings.filter((mapping) => mapping.action !== "skip").length;

  return (
    <form
      className="space-y-3"
      onSubmit={(event) => {
        event.preventDefault();
        void onSubmit(mappings);
      }}
    >
      <p className="text-sm text-muted-foreground">
        {t("slackImport.summary", { channels: channels.length, users: userCount })}
      </p>
      <ul className="divide-y rounded-md border">
        {channels.map((channel, index) => {
          const mapping = mappings[index];
          // 非公開チャンネルの履歴は、メンバー以外も読める公開チャンネルには入れない
          const choices = channel.is_private
            ? targets.filter(
                (target) => target.is_private || target.id === channel.existing_channel?.id,
              )
            : targets;
          const id = `slack-import-${channel.slack_channel_id}`;
          return (
            <li key={channel.slack_channel_id} className="flex flex-wrap items-center gap-2 p-3">
              <div className="min-w-0 flex-1 basis-40">
                <p className="flex items-center gap-1 text-sm font-medium break-all">
                  {channel.is_private ? (
                    <LockIcon className="size-3.5 shrink-0" aria-label={t("slackImport.private")} />
                  ) : null}
                  #{channel.name}
                </p>
                <p className="text-xs text-muted-foreground">
                  {t("slackImport.channelStats", {
                    messages: channel.message_count,
                    members: channel.members,
                  })}
                </p>
                {channel.existing_channel ? (
                  <p className="text-xs text-muted-foreground">
                    {t("slackImport.importedTo", { name: channel.existing_channel.name })}
                  </p>
                ) : null}
                {channel.deleted_channel ? (
                  <p className="text-xs text-destructive">
                    {t("slackImport.deletedTarget", { name: channel.deleted_channel })}
                  </p>
                ) : null}
              </div>
              <Select
                aria-label={t("slackImport.actionFor", { name: channel.name })}
                value={mapping.action}
                disabled={pending}
                onChange={(event) =>
                  update(
                    index,
                    mappingFor(
                      channel,
                      event.target.value as SlackChannelMapping["action"],
                      choices,
                    ),
                  )
                }
              >
                {/* 以前取り込んだチャンネルは同じ取り込み先にしか入れられない (メッセージ ID が変わらないため) */}
                <option
                  value="create"
                  disabled={!!channel.existing_channel || !!channel.deleted_channel}
                >
                  {t("slackImport.actionCreate")}
                </option>
                <option
                  value="existing"
                  disabled={choices.length === 0 || !!channel.deleted_channel}
                >
                  {t("slackImport.actionExisting")}
                </option>
                <option value="skip">{t("slackImport.actionSkip")}</option>
              </Select>
              {mapping.action === "create" ? (
                <Input
                  id={id}
                  aria-label={t("slackImport.channelName", { name: channel.name })}
                  className="w-48 max-w-full"
                  value={mapping.name}
                  disabled={pending}
                  onChange={(event) => update(index, { ...mapping, name: event.target.value })}
                />
              ) : null}
              {mapping.action === "existing" ? (
                <Select
                  aria-label={t("slackImport.targetChannel", { name: channel.name })}
                  className="w-48"
                  value={mapping.channel_id}
                  disabled={pending || !!channel.existing_channel}
                  onChange={(event) =>
                    update(index, { ...mapping, channel_id: event.target.value })
                  }
                >
                  {choices.map((target) => (
                    <option key={target.id} value={target.id}>
                      #{target.channel_name}
                    </option>
                  ))}
                </Select>
              ) : null}
            </li>
          );
        })}
      </ul>
      <Button type="submit" disabled={pending || selected === 0}>
        {t("slackImport.start", { count: selected })}
      </Button>
    </form>
  );
}

function Progress({ label, value }: { label: string; value: number }) {
  const percent = Math.round(Math.min(1, Math.max(0, value)) * 100);
  return (
    <div className="space-y-1">
      <p className="text-sm text-muted-foreground">{label}</p>
      <div
        role="progressbar"
        aria-label={label}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={percent}
        className="h-2 overflow-hidden rounded-full bg-muted"
      >
        <div className="h-full bg-primary transition-[width]" style={{ width: `${percent}%` }} />
      </div>
    </div>
  );
}

/** 取り込みのジョブは、どの ZIP からの取り込みかを名前に出す */
function jobTitle(job: SlackImport, t: TFunction): string {
  return isSlackExport(job) ? job.file_name : t("slackImport.runTitle", { name: job.file_name });
}

export function describeImportError(cause: unknown, t: TFunction): string {
  if (cause instanceof ApiError) {
    if (cause.status === 403) return t("slackImport.forbidden");
    const body = cause.body;
    if (body && typeof body === "object" && "error" in body) {
      const message = (body as { error: unknown }).error;
      if (typeof message === "string" && message) return message;
    }
  }
  return t("slackImport.failed");
}
