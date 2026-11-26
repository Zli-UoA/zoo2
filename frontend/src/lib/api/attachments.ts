import {
  attachmentFileName,
  resolveAttachmentContentType,
  type AttachmentRejection,
} from "../attachments";
import type { ApiClient } from "./client";
import type { PresignAttachmentResponse } from "./types";

export class AttachmentUploadError extends Error {
  readonly reason: AttachmentRejection | "request";

  constructor(reason: AttachmentRejection | "request", cause?: unknown) {
    super(reason === "request" ? "Attachment upload failed" : `Attachment rejected: ${reason}`, {
      cause,
    });
    this.name = "AttachmentUploadError";
    this.reason = reason;
  }
}

export type AttachmentDownloadErrorReason = "url" | "network" | "http";

/** ダウンロードの失敗。`url`（URL を取得できない）、`network`、`http` を区別する。 */
export class AttachmentDownloadError extends Error {
  readonly reason: AttachmentDownloadErrorReason;
  readonly status: number | null;

  constructor(
    reason: AttachmentDownloadErrorReason,
    options: { status?: number | null; cause?: unknown } = {},
  ) {
    super(
      reason === "url"
        ? "Attachment URL is unavailable"
        : reason === "http"
          ? `Attachment download failed with status ${options.status}`
          : "Attachment download failed",
      { cause: options.cause },
    );
    this.name = "AttachmentDownloadError";
    this.reason = reason;
    this.status = options.status ?? null;
  }
}

function errorStatus(error: unknown): number | null {
  if (typeof error !== "object" || error === null || !("status" in error)) {
    return null;
  }
  const status = (error as { status?: unknown }).status;
  return typeof status === "number" ? status : null;
}

/** `/url` の取得失敗を、API の状態コードを保ったダウンロードエラーにする。 */
export function attachmentUrlError(error: unknown): AttachmentDownloadError {
  return new AttachmentDownloadError("url", { status: errorStatus(error), cause: error });
}

export interface UploadedAttachment {
  id: string;
  file_name: string;
  file_size: number;
  content_type: string;
}

export async function uploadAttachment(
  client: ApiClient,
  channelId: string,
  file: File,
  fetchImpl: typeof fetch = fetch,
): Promise<UploadedAttachment> {
  const contentType = resolveAttachmentContentType(file);
  if (!contentType) {
    throw new AttachmentUploadError("unsupported");
  }
  const fileName = attachmentFileName(file, contentType);

  let presigned: PresignAttachmentResponse;
  try {
    const { data } = await client.post<PresignAttachmentResponse>("/api/attachments/presign", {
      channel_id: channelId,
      file_name: fileName,
      file_size: file.size,
      content_type: contentType,
    });
    presigned = data;
  } catch (error) {
    throw new AttachmentUploadError("request", error);
  }

  let response: Response;
  try {
    response = await fetchImpl(presigned.upload_url, {
      method: "PUT",
      headers: { "Content-Type": contentType },
      body: file,
    });
  } catch (error) {
    throw new AttachmentUploadError("request", error);
  }
  if (!response.ok) {
    throw new AttachmentUploadError("request");
  }

  return {
    id: presigned.attachment_id,
    file_name: fileName,
    file_size: file.size,
    content_type: contentType,
  };
}

export async function fetchAttachmentBlob(
  url: string,
  fetchImpl: typeof fetch = fetch,
): Promise<Blob> {
  try {
    const response = await fetchImpl(url, { credentials: "omit" });
    if (!response.ok) {
      throw new AttachmentDownloadError("http", { status: response.status });
    }
    return await response.blob();
  } catch (error) {
    // 通信そのものの失敗（オフライン、CORS、中断）と、200 受信後の本文読み取り失敗を
    // まとめて network として残す。非成功応答の http 分類はそのまま通す。
    if (error instanceof AttachmentDownloadError) {
      throw error;
    }
    throw new AttachmentDownloadError("network", { cause: error });
  }
}

/** 取得済みの `Blob` をブラウザーの保存として書き出す（デスクトップ版の保存が使えないとき）。 */
export function saveBlob(blob: Blob, fileName: string): void {
  const objectUrl = URL.createObjectURL(blob);
  try {
    const anchor = document.createElement("a");
    anchor.href = objectUrl;
    anchor.download = fileName;
    anchor.rel = "noopener";
    document.body.append(anchor);
    anchor.click();
    anchor.remove();
  } finally {
    window.setTimeout(() => URL.revokeObjectURL(objectUrl), 1_000);
  }
}

export type AttachmentUrlResolver = (options: { refresh: boolean }) => Promise<string>;

const DOWNLOAD_RETRY_DELAY_MS = 300;

function waitForRetry(delayMs: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, delayMs));
}

/**
 * ダウンロード用に URL を解決して中身を取得する。失敗したら URL を取り直し、1回だけ再試行する。
 * 開いた直後の一時的な `/url` の失敗や、署名付き URL の期限切れで保存できないのを防ぐ（#640）。
 */
export async function fetchAttachmentWithRetry(
  resolveUrl: AttachmentUrlResolver,
  options: { fetchImpl?: typeof fetch; retryDelayMs?: number } = {},
): Promise<Blob> {
  const { fetchImpl = fetch, retryDelayMs = DOWNLOAD_RETRY_DELAY_MS } = options;
  const attempt = async (refresh: boolean) => {
    const url = await resolveUrl({ refresh });
    return fetchAttachmentBlob(url, fetchImpl);
  };
  try {
    return await attempt(false);
  } catch {
    await waitForRetry(retryDelayMs);
    return attempt(true);
  }
}

/** ダウンロード用に URL を解決し、失敗時は URL を取り直して1回だけ保存を試し直す。 */
export async function downloadAttachmentWithRetry(
  resolveUrl: AttachmentUrlResolver,
  fileName: string,
  options: { fetchImpl?: typeof fetch; retryDelayMs?: number } = {},
): Promise<void> {
  saveBlob(await fetchAttachmentWithRetry(resolveUrl, options), fileName);
}
