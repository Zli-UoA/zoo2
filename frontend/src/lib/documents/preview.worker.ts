import { parseDocument, PreviewLimitError } from "./parse";
import { fetchPreviewBytes } from "./fetch";
import { type DocumentRequest, type DocumentResponse } from "./types";

self.onmessage = async (event: MessageEvent<DocumentRequest>) => {
  const { src, name, contentType } = event.data;
  try {
    const bytes = await fetchPreviewBytes(src);
    const data = await parseDocument(bytes, name, contentType);
    const transfer = [...new Set(data.files.map((file) => file.bytes.buffer))];
    self.postMessage({ data } satisfies DocumentResponse, { transfer });
  } catch (error) {
    self.postMessage({
      error: error instanceof PreviewLimitError ? "limit" : "invalid",
    } satisfies DocumentResponse);
  }
};
