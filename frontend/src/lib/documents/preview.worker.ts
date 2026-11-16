import { parseDocument, PreviewLimitError } from "./parse";
import { fetchPreviewBytes } from "./fetch";
import { type DocumentRequest, type DocumentResponse } from "./types";

self.onmessage = async (event: MessageEvent<DocumentRequest>) => {
  const { src, name, contentType } = event.data;
  try {
    const bytes = await fetchPreviewBytes(src);
    const data = await parseDocument(bytes, name, contentType);
    self.postMessage({ data } satisfies DocumentResponse);
  } catch (error) {
    self.postMessage({
      error: error instanceof PreviewLimitError ? "limit" : "invalid",
    } satisfies DocumentResponse);
  }
};
