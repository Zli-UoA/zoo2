import { MAX_PREVIEW_BYTES } from "./types";

export class PreviewLimitError extends Error {}

export async function fetchPreviewBytes(
  src: string,
  signal?: AbortSignal,
): Promise<Uint8Array<ArrayBuffer>> {
  const response = await fetch(src, { credentials: "omit", referrerPolicy: "no-referrer", signal });
  if (!response.ok) throw new Error("Preview request failed");
  if (Number(response.headers.get("content-length")) > MAX_PREVIEW_BYTES) {
    await response.body?.cancel();
    throw new PreviewLimitError();
  }
  const reader = response.body?.getReader();
  if (!reader) throw new Error("Missing response body");
  const chunks: Uint8Array[] = [];
  let size = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      size += value.byteLength;
      if (size > MAX_PREVIEW_BYTES) throw new PreviewLimitError();
      chunks.push(value);
    }
  } finally {
    await reader.cancel();
  }
  const bytes = new Uint8Array(size);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return bytes;
}
