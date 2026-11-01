import type { ApiClient } from "./client";

/** バックエンドの `slack_import_jobs.status` */
export type SlackImportStatus =
  | "uploading"
  | "analyzing"
  | "awaiting_mapping"
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
  message_count: number;
  /** zoo で使える名前の候補。予約語や既存チャンネルと重なると `slack-` を付ける */
  suggested_name: string;
  name_taken: boolean;
  /** 以前の取り込みで対応付けた zoo チャンネル */
  existing_channel: { id: string; name: string } | null;
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
  error: string | null;
  created_at: string;
  finished_at: string | null;
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

/** 解析結果から、初期状態の対応付けを作る */
export function defaultMappings(channels: SlackChannelSummary[]): SlackChannelMapping[] {
  return channels.map((channel) =>
    channel.existing_channel
      ? {
          slack_channel_id: channel.slack_channel_id,
          action: "existing",
          channel_id: channel.existing_channel.id,
        }
      : {
          slack_channel_id: channel.slack_channel_id,
          action: "create",
          name: channel.suggested_name,
          is_private: channel.is_private,
        },
  );
}
