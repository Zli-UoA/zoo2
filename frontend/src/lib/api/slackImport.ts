import type { ApiClient } from "./client";

/**
 * バックエンドの `slack_import_jobs.status`。
 * エクスポート (アップロードした ZIP) は uploading → analyzing → ready で、管理者が消すか新しい ZIP に
 * 置き換わるまで ready のまま残る。取り込みのたびに、エクスポートを `source_id` に持つジョブを作り、
 * importing → fetching_files → completed と進む。
 */
export type SlackImportStatus =
  | "uploading"
  | "analyzing"
  | "ready"
  | "importing"
  | "fetching_files"
  | "completed"
  | "failed"
  | "cancelled";

export interface SlackChannelSummary {
  slack_channel_id: string;
  name: string;
  is_private: boolean;
  is_archived: boolean;
  members: number;
  /** チャンネルを作った Slack ユーザー */
  creator: string;
  message_count: number;
  /** zoo で使える名前の候補。予約語や既存チャンネルと重なると `slack-` を付ける */
  suggested_name: string;
  name_taken: boolean;
  /** 以前の取り込みで対応付けた zoo チャンネル */
  existing_channel: {
    id: string;
    name: string;
    is_private?: boolean;
    /** 最初の取り込みの種類 (新しく作った / 既存のチャンネルに追加) と、最後に取り込んだ日時 */
    import_kind?: "created" | "existing" | null;
    imported_at?: string | null;
  } | null;
  /**
   * 以前の取り込み先が、見えないチャンネル (参加していない非公開チャンネル。ゲストには公開チャンネルも)。
   * 名前と ID は返さない。
   * 取り込めるのはそこだけなので、新しく作ることもできない (各ユーザーによる取り込み)
   */
  existing_hidden?: boolean;
  /** Slack と同じ名前の zoo チャンネル */
  same_name_channel?: { id: string; name: string; is_private: boolean } | null;
  /** 以前の取り込み先が削除済みならその名前。過去ログが残っているため取り込み直せない */
  deleted_channel?: string | null;
}

export interface SlackImport {
  id: string;
  status: SlackImportStatus;
  file_name: string;
  file_size: number;
  team_id: string | null;
  analysis: { team_id: string; user_count: number; channels: SlackChannelSummary[] } | null;
  total_messages: number;
  processed_messages: number;
  total_files: number;
  processed_files: number;
  pending_files: number;
  /** 取得に 3 回失敗した添付。「添付ファイルを取得」で再取得できる */
  failed_files: number;
  /** 再取得でやり直せるスキップ (サイズの上限・取得を許可する URL の設定が理由のもの) */
  skipped_files?: number;
  error: string | null;
  created_at: string;
  finished_at: string | null;
  /** 取り込みのジョブなら、ZIP を読むエクスポートの ID。エクスポート自身は null */
  source_id: string | null;
}

export type SlackChannelMapping =
  | {
      slack_channel_id: string;
      action: "create";
      name: string;
      is_private: boolean;
      /** 取り込みが終わったら、作ったチャンネルをアーカイブする (管理者の取り込みだけ) */
      archive?: boolean;
    }
  | { slack_channel_id: string; action: "existing"; channel_id: string }
  | { slack_channel_id: string; action: "skip" };

/** ワーカーが処理中で、結果を待つ必要がある状態 */
export const SLACK_IMPORT_RUNNING: readonly SlackImportStatus[] = [
  "uploading",
  "analyzing",
  "importing",
  "fetching_files",
];

const BASE = "/api/admin/slack-imports";

export function listSlackImports(client: ApiClient, signal?: AbortSignal): Promise<SlackImport[]> {
  return client.get<{ imports: SlackImport[] }>(BASE, { signal }).then((value) => value.imports);
}

export function getSlackImport(
  client: ApiClient,
  id: string,
  signal?: AbortSignal,
): Promise<SlackImport> {
  return client.get<SlackImport>(`${BASE}/${id}`, { signal });
}

export function startSlackImport(
  client: ApiClient,
  id: string,
  mappings: SlackChannelMapping[],
): Promise<SlackImport> {
  return client.request<SlackImport>(`${BASE}/${id}/start`, {
    method: "POST",
    body: { mappings },
  });
}

export function cancelSlackImport(client: ApiClient, id: string): Promise<SlackImport> {
  return client.request<SlackImport>(`${BASE}/${id}`, { method: "DELETE" });
}

export function fetchSlackImportFiles(client: ApiClient, id: string): Promise<SlackImport> {
  return client.request<SlackImport>(`${BASE}/${id}/fetch-files`, { method: "POST" });
}

export class SlackExportUploadError extends Error {
  constructor(cause?: unknown) {
    super("Slack export upload failed", { cause });
    this.name = "SlackExportUploadError";
  }
}

/** presigned URL へ PUT する。数 GB になるので進み具合を通知できる XHR を使う。 */
export type PutWithProgress = (
  url: string,
  file: Blob,
  headers: Record<string, string>,
  onProgress: (loaded: number, total: number) => void,
  signal?: AbortSignal,
) => Promise<void>;

export const xhrPut: PutWithProgress = (url, file, headers, onProgress, signal) =>
  new Promise((resolve, reject) => {
    const xhr = new XMLHttpRequest();
    xhr.open("PUT", url);
    for (const [name, value] of Object.entries(headers)) {
      xhr.setRequestHeader(name, value);
    }
    xhr.upload.onprogress = (event) => {
      if (event.lengthComputable) onProgress(event.loaded, event.total);
    };
    xhr.onload = () =>
      xhr.status >= 200 && xhr.status < 300
        ? resolve()
        : reject(new SlackExportUploadError(xhr.status));
    xhr.onerror = () => reject(new SlackExportUploadError());
    xhr.onabort = () => reject(new SlackExportUploadError("aborted"));
    signal?.addEventListener("abort", () => xhr.abort(), { once: true });
    xhr.send(file);
  });

/** ジョブを作り、ZIP を R2 に置いてから解析を依頼する */
export async function uploadSlackExport(
  client: ApiClient,
  file: File,
  onProgress: (ratio: number) => void,
  put: PutWithProgress = xhrPut,
  signal?: AbortSignal,
  /** ジョブを作ったとき (アップロードの前) に呼ぶ。取り消したジョブのアップロードを中断するのに使う */
  onCreated?: (job: SlackImport) => void,
): Promise<SlackImport> {
  const created = await client.request<{
    import: SlackImport;
    upload_url: string;
    upload_headers: Record<string, string>;
  }>(BASE, { method: "POST", body: { file_name: file.name, file_size: file.size } });
  onCreated?.(created.import);
  await put(
    created.upload_url,
    file,
    created.upload_headers,
    (loaded, total) => onProgress(total > 0 ? loaded / total : 0),
    signal,
  );
  return client.request<SlackImport>(`${BASE}/${created.import.id}/uploaded`, { method: "POST" });
}

/**
 * 操作のエラーを消してよいかを決めるための、一覧の状態の要約 (ジョブの ID と状態の組)。
 * 進行中のジョブの件数 (`processed_messages` など) は 2 秒ごとに変わるので含めない
 */
export function slackImportsSnapshot(imports: SlackImport[]): string {
  return JSON.stringify(imports.map((job) => [job.id, job.status]));
}

/** エクスポート (アップロードした ZIP) か。取り込みのジョブは `source_id` を持つ */
export function isSlackExport(job: SlackImport): boolean {
  return !job.source_id;
}

/**
 * カードで操作できるように出すものと、履歴に回すものに分ける。
 * 保存中・保存済みのエクスポートと、進行中・失敗した取り込みはすべてカードにする
 * (別の管理者やタブが始めたものも操作できるように)。
 * 進行中の取り込みがなければ、最新の完了した取り込みの結果もカードで出す。
 */
export function splitSlackImports(imports: SlackImport[]): {
  cards: SlackImport[];
  history: SlackImport[];
} {
  const exports = imports.filter(
    (job) => isSlackExport(job) && job.status !== "cancelled" && job.status !== "completed",
  );
  const runs = imports.filter(
    (job) => !isSlackExport(job) && job.status !== "completed" && job.status !== "cancelled",
  );
  const latest = imports.find((job) => !isSlackExport(job) && job.status === "completed");
  const cards = [...exports, ...(runs.length > 0 ? runs : latest ? [latest] : [])];
  return { cards, history: imports.filter((job) => !cards.includes(job)) };
}

/** 「添付ファイルを取得」でやり直せる件数 */
export function retryableFiles(job: SlackImport): number {
  return job.status === "completed"
    ? job.pending_files + job.failed_files + (job.skipped_files ?? 0)
    : 0;
}

/** 保存した ZIP から取り込むチャンネルは管理者が選ぶので、初めはどれも取り込まない */
export function defaultMappings(channels: SlackChannelSummary[]): SlackChannelMapping[] {
  return channels.map((channel) => ({
    slack_channel_id: channel.slack_channel_id,
    action: "skip",
  }));
}

/**
 * 取り込み方を選んだときの対応付け。以前取り込んだチャンネルは同じ取り込み先に固定する
 * (メッセージ ID が変わらないため)。`targets` は選べる既存チャンネル。
 */
/** 「既存のチャンネル」で選べる取り込み先 */
export type SlackTargetChoice = { id: string; channel_name: string; is_private: boolean };

/**
 * 「既存のチャンネル」で選べる取り込み先。非公開チャンネルの履歴は公開チャンネルに入れない。
 * 以前の取り込み先は、参加していない非公開チャンネルで同期されていなくても選べるよう先頭に足す
 * (以前取り込んだチャンネルはそこにしか入れられないため)。
 */
export function targetChoices(
  channel: SlackChannelSummary,
  targets: SlackTargetChoice[],
): SlackTargetChoice[] {
  const existing = channel.existing_channel;
  const choices = channel.is_private
    ? targets.filter((target) => target.is_private || target.id === existing?.id)
    : targets;
  if (!existing || choices.some((target) => target.id === existing.id)) return choices;
  return [
    {
      id: existing.id,
      channel_name: existing.name,
      is_private: existing.is_private ?? channel.is_private,
    },
    ...choices,
  ];
}

export function mappingFor(
  channel: SlackChannelSummary,
  action: SlackChannelMapping["action"],
  targets: { id: string }[],
): SlackChannelMapping {
  const base = { slack_channel_id: channel.slack_channel_id };
  if (action === "skip") return { ...base, action };
  if (action === "existing")
    return { ...base, action, channel_id: channel.existing_channel?.id ?? targets[0]?.id ?? "" };
  return { ...base, action, name: channel.suggested_name, is_private: channel.is_private };
}

/** 以前に取り込んだ (または取り込み先が削除された) チャンネルは、取り込み方を変えられない */
function isFixedTarget(channel: SlackChannelSummary): boolean {
  return !!channel.existing_channel || !!channel.existing_hidden || !!channel.deleted_channel;
}

/**
 * 一括で「新しいチャンネル」か「取り込まない」にする。以前に取り込んだチャンネルは同じ取り込み先に固定なので、
 * 「新しいチャンネル」への一括変更では今の指定のまま変えない。すでに「新しいチャンネル」の指定は、
 * 編集した名前や設定を残す。
 */
export function applyBulkAction(
  channels: SlackChannelSummary[],
  mappings: SlackChannelMapping[],
  action: "create" | "skip",
): SlackChannelMapping[] {
  return mappings.map((current, index) => {
    const channel = channels[index];
    if (action === "skip") return { slack_channel_id: current.slack_channel_id, action: "skip" };
    if (isFixedTarget(channel) || current.action === "create") return current;
    return mappingFor(channel, "create", []);
  });
}

/** 「新しいチャンネル」の指定すべてに、取り込み後のアーカイブを指定する (外す) */
export function applyBulkArchive(
  mappings: SlackChannelMapping[],
  archive: boolean,
): SlackChannelMapping[] {
  return mappings.map((mapping) =>
    mapping.action === "create" ? { ...mapping, archive } : mapping,
  );
}
