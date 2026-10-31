import type { AttachmentRow } from "./electric/types";
import { DEFAULT_LOCALE, INTL_LOCALES, type Locale } from "./i18n/locale";

export const MAX_ATTACHMENTS_PER_MESSAGE = 10;
export const MAX_ATTACHMENT_FILE_SIZE = 50 * 1024 * 1024;

export const SUPPORTED_IMAGE_TYPES = [
  "image/png",
  "image/jpeg",
  "image/webp",
  "image/gif",
] as const;

const EXTENSION_TO_IMAGE_TYPE: Record<string, string> = {
  png: "image/png",
  jpg: "image/jpeg",
  jpeg: "image/jpeg",
  webp: "image/webp",
  gif: "image/gif",
};

const IMAGE_TYPE_TO_EXTENSION: Record<string, string> = {
  "image/png": "png",
  "image/jpeg": "jpg",
  "image/webp": "webp",
  "image/gif": "gif",
};

export type AttachmentRejection = "unsupported" | "too-many" | "too-large";

export function isInlineImage(contentType: string): boolean {
  const mime = contentType.split(";")[0]?.trim().toLowerCase() ?? "";
  return (SUPPORTED_IMAGE_TYPES as readonly string[]).includes(mime);
}

export function resolveSupportedImageType(file: Pick<File, "type" | "name">): string | null {
  const declared = file.type.split(";")[0]?.trim().toLowerCase() ?? "";
  if ((SUPPORTED_IMAGE_TYPES as readonly string[]).includes(declared)) {
    return declared;
  }
  const extension = file.name.split(".").pop()?.toLowerCase() ?? "";
  return EXTENSION_TO_IMAGE_TYPE[extension] ?? null;
}

export function imageFilesFromClipboard(
  data: Pick<DataTransfer, "items" | "files"> | null,
): File[] {
  if (!data) {
    return [];
  }
  const candidates = [...(data.files ?? [])];
  for (const item of data.items ?? []) {
    if (item.kind !== "file") {
      continue;
    }
    const file = item.getAsFile();
    if (file) {
      candidates.push(file);
    }
  }
  const seen = new Set<string>();
  return candidates.filter((file) => {
    if (!resolveSupportedImageType(file)) {
      return false;
    }
    const key = `${file.name}:${file.size}:${file.type}:${file.lastModified}`;
    if (seen.has(key)) {
      return false;
    }
    seen.add(key);
    return true;
  });
}

export function attachmentFileName(file: Pick<File, "name">, contentType: string): string {
  const cleaned = Array.from(file.name)
    .map((char) => (char.charCodeAt(0) < 32 || char.charCodeAt(0) === 127 ? "_" : char))
    .join("")
    .trim();
  if (cleaned) {
    return Array.from(cleaned).slice(0, 255).join("");
  }
  const extension = IMAGE_TYPE_TO_EXTENSION[contentType] ?? "png";
  const suffix = Math.random().toString(36).slice(2, 8);
  return `pasted-image-${Date.now()}-${suffix}.${extension}`;
}

export function attachmentRejection(
  file: Pick<File, "type" | "name" | "size">,
  currentCount: number,
): AttachmentRejection | null {
  if (currentCount >= MAX_ATTACHMENTS_PER_MESSAGE) {
    return "too-many";
  }
  if (file.size > MAX_ATTACHMENT_FILE_SIZE) {
    return "too-large";
  }
  if (!resolveSupportedImageType(file)) {
    return "unsupported";
  }
  return null;
}

export function formatAttachmentSize(
  bytes: number | string,
  locale: Locale = DEFAULT_LOCALE,
): string {
  const value = typeof bytes === "number" ? bytes : Number(bytes);
  if (!Number.isFinite(value) || value < 0) {
    return "—";
  }
  const units = ["B", "KB", "MB", "GB"];
  let size = value;
  let unit = 0;
  while (size >= 1024 && unit < units.length - 1) {
    size /= 1024;
    unit += 1;
  }
  const formatted = new Intl.NumberFormat(INTL_LOCALES[locale], {
    maximumFractionDigits: unit === 0 ? 0 : 1,
  }).format(size);
  return `${formatted} ${units[unit]}`;
}

export function groupAttachmentsByMessage(
  rows: readonly AttachmentRow[],
): Map<string, AttachmentRow[]> {
  const grouped = new Map<string, AttachmentRow[]>();
  for (const row of [...rows].sort((a, b) => a.created_at.localeCompare(b.created_at))) {
    if (!row.message_id) {
      continue;
    }
    const bucket = grouped.get(row.message_id);
    if (bucket) {
      bucket.push(row);
    } else {
      grouped.set(row.message_id, [row]);
    }
  }
  return grouped;
}
