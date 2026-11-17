export const MAX_PREVIEW_BYTES = 12 * 1024 * 1024;
export const MAX_EXPANDED_BYTES = 16 * 1024 * 1024;
export const MAX_PREVIEW_FILES = 100;
export const MAX_PREVIEW_TEXT = 500_000;
export interface PreviewFile {
  name: string;
  type: string;
  bytes: Uint8Array<ArrayBuffer>;
}
export interface DocumentPreviewData {
  headers?: { label: "subject" | "from" | "to" | "cc" | "date"; value: string }[];
  sections: { name: string; text: string }[];
  files: PreviewFile[];
}
export type DocumentRequest = { src: string; name: string; contentType: string };
export type DocumentResponse = { data: DocumentPreviewData } | { error: "limit" | "invalid" };
