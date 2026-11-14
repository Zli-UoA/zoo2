import type { AttachmentInfo, EncryptedFile } from "@zoo/crypto-wasm";

import { AttachmentUploadError } from "../api/attachments";
import type { ApiClient } from "../api/client";
import type { AttachmentUrlResponse, PresignAttachmentResponse } from "../api/types";
import { attachmentFileName, resolveAttachmentContentType } from "../attachments";
import type { CryptoWorker } from "./cryptoWorker";

/**
 * 暗号化チャンネルの添付（#364。docs/e2ee.md の「暗号化した添付」）。
 *
 * ファイルはクライアントで AES-256-CTR で暗号化してから S3 に上げ、鍵、IV、ハッシュと本当のファイル名、
 * MIME、画像の大きさは、Megolm で暗号化する本文の `attachments`（`AttachmentInfo`）に入れる。
 * サーバーには、決まったファイル名と Content-Type（zoo-crypto の `ENCRYPTED_ATTACHMENT_*`）と暗号文の
 * 大きさだけを渡す。
 */

/** zoo-crypto の `ENCRYPTED_ATTACHMENT_FILE_NAME`。 */
export const ENCRYPTED_ATTACHMENT_FILE_NAME = "encrypted.bin";
/** zoo-crypto の `ENCRYPTED_ATTACHMENT_CONTENT_TYPE`。 */
export const ENCRYPTED_ATTACHMENT_CONTENT_TYPE = "application/octet-stream";

/** 暗号化と復号（ブラウザーでは暗号の Worker）。 */
export interface AttachmentCrypto {
  encrypt(plaintext: Uint8Array): Promise<{ ciphertext: Uint8Array; file: EncryptedFile }>;
  decrypt(file: EncryptedFile, ciphertext: Uint8Array): Promise<Uint8Array>;
}

/** 暗号の Worker で暗号化と復号をする。 */
export function workerAttachmentCrypto(worker: CryptoWorker): AttachmentCrypto {
  return {
    encrypt: (plaintext) => worker.request({ op: "encryptAttachment", plaintext }),
    decrypt: (file, ciphertext) => worker.request({ op: "decryptAttachment", file, ciphertext }),
  };
}

/** 画像の大きさ。読めなければ null（本文から省く）。 */
export type ImageSizeReader = (
  file: Blob,
  contentType: string,
) => Promise<{ width: number; height: number } | null>;

/** `createImageBitmap` で画像の大きさを読む。画像でない、または読めなければ null。 */
export const readImageSize: ImageSizeReader = async (file, contentType) => {
  if (!contentType.startsWith("image/") || typeof createImageBitmap !== "function") return null;
  try {
    const bitmap = await createImageBitmap(file);
    const size = { width: bitmap.width, height: bitmap.height };
    bitmap.close();
    return size.width > 0 && size.height > 0 ? size : null;
  } catch {
    return null;
  }
};

/**
 * `file` を暗号化して上げ、本文の `attachments` に入れる `AttachmentInfo` を返す。
 *
 * 送信（`attachment_ids`）にはこの `attachment_id` を、本文にはこの `AttachmentInfo` を入れる。
 * 平文はサーバーにも S3 にも渡さない。
 */
export async function uploadEncryptedAttachment(
  client: ApiClient,
  channelId: string,
  file: File,
  crypto: AttachmentCrypto,
  options: { fetchImpl?: typeof fetch; readSize?: ImageSizeReader; sensitive?: boolean } = {},
): Promise<AttachmentInfo> {
  const fetchImpl = options.fetchImpl ?? fetch;
  const contentType = resolveAttachmentContentType(file);
  if (!contentType) {
    throw new AttachmentUploadError("unsupported");
  }
  const name = attachmentFileName(file, contentType);
  const plaintext = new Uint8Array(await file.arrayBuffer());
  const size = await (options.readSize ?? readImageSize)(file, contentType);
  const { ciphertext, file: encryptedFile } = await crypto.encrypt(plaintext);

  let presigned: PresignAttachmentResponse;
  try {
    const { data } = await client.post<PresignAttachmentResponse>("/api/attachments/presign", {
      channel_id: channelId,
      file_name: ENCRYPTED_ATTACHMENT_FILE_NAME,
      file_size: ciphertext.byteLength,
      content_type: ENCRYPTED_ATTACHMENT_CONTENT_TYPE,
    });
    presigned = data;
  } catch (error) {
    throw new AttachmentUploadError("request", error);
  }

  let response: Response;
  try {
    response = await fetchImpl(presigned.upload_url, {
      method: "PUT",
      headers: { "Content-Type": ENCRYPTED_ATTACHMENT_CONTENT_TYPE },
      body: new Blob([ciphertext as BlobPart], { type: ENCRYPTED_ATTACHMENT_CONTENT_TYPE }),
    });
  } catch (error) {
    throw new AttachmentUploadError("request", error);
  }
  if (!response.ok) {
    throw new AttachmentUploadError("request");
  }

  return {
    attachment_id: presigned.attachment_id,
    name,
    mimetype: contentType,
    size: plaintext.byteLength,
    ...(size ? { width: size.width, height: size.height } : {}),
    ...(options.sensitive ? { sensitive: true } : {}),
    file: encryptedFile,
  };
}

/** 暗号化した添付を取得できない、または復号できない（ハッシュが合わないなど）。 */
export class EncryptedAttachmentError extends Error {
  constructor(
    readonly reason: "download" | "decrypt",
    cause?: unknown,
  ) {
    super(`Encrypted attachment ${reason} failed`, { cause });
    this.name = "EncryptedAttachmentError";
  }
}

/**
 * 本文の `AttachmentInfo` の添付を取得して復号し、元の MIME の `Blob` にする。
 *
 * 暗号文のハッシュが本文の値と合わなければ投げる（サーバーや S3 が中身を差し替えても、平文を返さない）。
 * `resolveUrl` を渡すと、署名付き URL をそこから得る（画面では `lib/api/attachmentUrls.ts` のキャッシュ）。
 */
export async function fetchEncryptedAttachment(
  client: ApiClient,
  info: AttachmentInfo,
  crypto: AttachmentCrypto,
  fetchImpl: typeof fetch = fetch,
  resolveUrl: (attachmentId: string) => Promise<string> = async (attachmentId) =>
    (
      await client.requestWithTx<AttachmentUrlResponse>(
        `/api/attachments/${encodeURIComponent(attachmentId)}/url`,
        { method: "GET" },
      )
    ).data.download_url,
): Promise<Blob> {
  let ciphertext: Uint8Array;
  try {
    const downloadUrl = await resolveUrl(info.attachment_id);
    const response = await fetchImpl(downloadUrl, { credentials: "omit" });
    if (!response.ok) {
      throw new Error(`Attachment download failed with status ${response.status}`);
    }
    ciphertext = new Uint8Array(await response.arrayBuffer());
  } catch (error) {
    throw new EncryptedAttachmentError("download", error);
  }
  let plaintext: Uint8Array;
  try {
    plaintext = await crypto.decrypt(info.file, ciphertext);
  } catch (error) {
    throw new EncryptedAttachmentError("decrypt", error);
  }
  // 本文の大きさと違えば、送信者の値を信じない（docs/e2ee.md。表示の大きさと中身を食い違わせない）。
  if (plaintext.byteLength !== info.size) {
    throw new EncryptedAttachmentError("decrypt");
  }
  return new Blob([plaintext as BlobPart], { type: receivedAttachmentView(info).mimetype });
}

/** 表示や保存の向きを変える双方向の制御文字（U+202E などで拡張子を偽装させない）。 */
const BIDI_CONTROLS = /[\u200e\u200f\u202a-\u202e\u2066-\u2069]/g;

/**
 * 受け取った添付を表示と保存に使う形にする。
 *
 * 暗号化チャンネルでは、サーバーは名前と MIME を見られないので、平文の添付でサーバーが拒否する種類
 * （`resolveAttachmentContentType` が null を返す実行ファイルなど）は、受信側で開かせない（`blocked`）。
 * 名前の双方向の制御文字は `_` に置き換える。
 */
export function receivedAttachmentView(info: Pick<AttachmentInfo, "name" | "mimetype">): {
  name: string;
  mimetype: string;
  blocked: boolean;
} {
  const name = info.name.replace(BIDI_CONTROLS, "_");
  const mimetype = resolveAttachmentContentType({ name, type: info.mimetype });
  return mimetype
    ? { name, mimetype, blocked: false }
    : { name, mimetype: ENCRYPTED_ATTACHMENT_CONTENT_TYPE, blocked: true };
}
