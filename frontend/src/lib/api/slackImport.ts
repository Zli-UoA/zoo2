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
  existing_channel: { id: string; name: string; is_private?: boolean } | null;
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
  error: string | null;
  created_at: string;
  finished_at: string | null;
  /** 取り込みのジョブなら、ZIP を読むエクスポートの ID。エクスポート自身は null */
  source_id: string | null;
}

export type SlackChannelMapping =
  | { slack_channel_id: string; action: "create"; name: string; is_private: boolean }
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
): Promise<SlackImport> {
  const created = await client.request<{
    import: SlackImport;
    upload_url: string;
    upload_headers: Record<string, string>;
  }>(BASE, { method: "POST", body: { file_name: file.name, file_size: file.size } });
  await put(
    created.upload_url,
    file,
    created.upload_headers,
    (loaded, total) => onProgress(total > 0 ? loaded / total : 0),
    signal,
  );
  return client.request<SlackImport>(`${BASE}/${created.import.id}/uploaded`, { method: "POST" });
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
  return job.status === "completed" ? job.pending_files + job.failed_files : 0;
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

/** 連携したユーザーが作った Slack チャンネル。`targets` は取り込み先に選べる既存チャンネル */
export interface OwnSlackChannel extends SlackChannelSummary {
  targets: { id: string; name: string; is_private: boolean }[];
}

/** 連携したユーザーが、保存済みの ZIP から自分のチャンネルを取り込むための情報 */
export interface OwnSlackImports {
  /** 取り込みに使う ZIP。管理者がまだ保存していなければ null */
  export: { id: string; file_name: string; created_at: string } | null;
  channels: OwnSlackChannel[];
  /** 自分が始めた取り込み (新しい順) */
  imports: SlackImport[];
}

export function getOwnSlackImports(
  client: ApiClient,
  signal?: AbortSignal,
): Promise<OwnSlackImports> {
  return client.get<OwnSlackImports>("/api/slack/imports", { signal });
}

export function startOwnSlackImport(
  client: ApiClient,
  mappings: SlackChannelMapping[],
): Promise<SlackImport> {
  return client.request<SlackImport>("/api/slack/imports", {
    method: "POST",
    body: { mappings },
  });
}
