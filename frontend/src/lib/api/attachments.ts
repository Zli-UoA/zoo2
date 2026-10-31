import {
  attachmentFileName,
  resolveSupportedImageType,
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
  const contentType = resolveSupportedImageType(file);
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
