import type { AttachmentRow } from "./electric/types";
import { DEFAULT_LOCALE, INTL_LOCALES, type Locale } from "./i18n/locale";

export const MAX_ATTACHMENTS_PER_MESSAGE = 10;
export const MAX_ATTACHMENT_FILE_SIZE = 50 * 1024 * 1024;
export const ATTACHMENT_URL_FALLBACK_TTL_MS = 4 * 60 * 1000;
export const ATTACHMENT_URL_REFRESH_MARGIN_MS = 5 * 1000;

const SUPPORTED_IMAGE_TYPES = ["image/png", "image/jpeg", "image/webp", "image/gif"] as const;

const EXTENSION_TO_IMAGE_TYPE: Record<string, string> = {
  png: "image/png",
  jpg: "image/jpeg",
  jpeg: "image/jpeg",
  webp: "image/webp",
  gif: "image/gif",
};

const EXTENSION_TO_CONTENT_TYPE: Record<string, string> = {
  ...EXTENSION_TO_IMAGE_TYPE,
  pdf: "application/pdf",
  txt: "text/plain",
  md: "text/markdown",
  csv: "text/csv",
  json: "application/json",
  xml: "application/xml",
  zip: "application/zip",
  gz: "application/gzip",
  tar: "application/x-tar",
  "7z": "application/x-7z-compressed",
  rar: "application/vnd.rar",
  doc: "application/msword",
  docx: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
  xls: "application/vnd.ms-excel",
  xlsx: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
  ppt: "application/vnd.ms-powerpoint",
  pptx: "application/vnd.openxmlformats-officedocument.presentationml.presentation",
  mp3: "audio/mpeg",
  m4a: "audio/mp4",
  wav: "audio/wav",
  ogg: "audio/ogg",
  flac: "audio/flac",
  mp4: "video/mp4",
  webm: "video/webm",
  mov: "video/quicktime",
};

const CONTENT_TYPE_TO_EXTENSION: Record<string, string> = {
  "image/png": "png",
  "image/jpeg": "jpg",
  "image/webp": "webp",
  "image/gif": "gif",
  "application/pdf": "pdf",
  "text/plain": "txt",
  "text/markdown": "md",
  "text/csv": "csv",
  "application/json": "json",
  "application/xml": "xml",
  "application/zip": "zip",
  "application/gzip": "gz",
  "application/x-tar": "tar",
  "application/x-7z-compressed": "7z",
  "application/vnd.rar": "rar",
  "application/msword": "doc",
  "application/vnd.openxmlformats-officedocument.wordprocessingml.document": "docx",
  "application/vnd.ms-excel": "xls",
  "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet": "xlsx",
  "application/vnd.ms-powerpoint": "ppt",
  "application/vnd.openxmlformats-officedocument.presentationml.presentation": "pptx",
  "audio/mpeg": "mp3",
  "audio/mp4": "m4a",
  "audio/wav": "wav",
  "audio/ogg": "ogg",
  "audio/flac": "flac",
  "video/mp4": "mp4",
  "video/webm": "webm",
  "video/quicktime": "mov",
};

const DISALLOWED_UPLOAD_TYPES = new Set([
  "application/x-msdownload",
  "application/x-executable",
  "application/x-dosexec",
  "application/x-msdos-program",
  "application/x-sharedlib",
  "application/x-batch",
  "application/x-sh",
]);

const DISALLOWED_UPLOAD_EXTENSIONS = new Set([
  "exe",
  "com",
  "scr",
  "msi",
  "dll",
  "so",
  "dylib",
  "bat",
  "cmd",
  "sh",
]);

export type AttachmentRejection = "unsupported" | "too-many" | "too-large" | "empty";

function baseMime(value: string): string {
  return value.split(";")[0]?.trim().toLowerCase() ?? "";
}

function fileExtension(name: string): string {
  return name.split(".").pop()?.toLowerCase() ?? "";
}

export function isInlineImage(contentType: string): boolean {
  return (SUPPORTED_IMAGE_TYPES as readonly string[]).includes(baseMime(contentType));
}

export function resolveAttachmentContentType(file: Pick<File, "type" | "name">): string | null {
  const extension = fileExtension(file.name);
  if (DISALLOWED_UPLOAD_EXTENSIONS.has(extension)) {
    return null;
  }
  const inferred = EXTENSION_TO_CONTENT_TYPE[extension];
  const declared = baseMime(file.type);
  if (
    declared === "application/octet-stream" ||
    declared === "binary/octet-stream" ||
    !/^[a-z0-9!#$&^_.+-]+\/[a-z0-9!#$&^_.+-]+$/.test(declared)
  ) {
    return inferred ?? "application/octet-stream";
  }
  return DISALLOWED_UPLOAD_TYPES.has(declared) ? null : declared;
}

export function attachmentFilesFromClipboard(
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
    if (!resolveAttachmentContentType(file)) {
      return false;
    }
    const key = `${file.name}:${file.size}:${file.type}`;
    if (seen.has(key)) {
      return false;
    }
    seen.add(key);
    return true;
  });
}

export function attachmentUrlExpiresAt(url: string, now = Date.now()): number {
  try {
    const parsed = new URL(url);
    const expiresParam = parsed.searchParams.get("X-Amz-Expires");
    const expires = expiresParam ? Number(expiresParam) : Number.NaN;
    if (!Number.isFinite(expires) || expires <= 0) {
      return now + ATTACHMENT_URL_FALLBACK_TTL_MS;
    }
    const dateParam = parsed.searchParams.get("X-Amz-Date");
    const normalizedDate = dateParam?.replace(
      /^(\d{4})(\d{2})(\d{2})T(\d{2})(\d{2})(\d{2})Z$/,
      "$1-$2-$3T$4:$5:$6Z",
    );
    const issuedAt = normalizedDate ? Date.parse(normalizedDate) : now;
    return (Number.isFinite(issuedAt) ? issuedAt : now) + expires * 1000;
  } catch {
    return now + ATTACHMENT_URL_FALLBACK_TTL_MS;
  }
}

export function shouldRefreshAttachmentUrl(expiresAt: number | null, now = Date.now()): boolean {
  return expiresAt === null || expiresAt - now <= ATTACHMENT_URL_REFRESH_MARGIN_MS;
}

function contentTypeExtension(contentType: string): string {
  const known = CONTENT_TYPE_TO_EXTENSION[baseMime(contentType)];
  if (known) {
    return known;
  }
  const subtype = baseMime(contentType).split("/")[1] ?? "";
  return subtype.replace(/[^a-z0-9]+/g, "").slice(0, 12) || "bin";
}

export function attachmentFileName(file: Pick<File, "name">, contentType: string): string {
  const cleaned = Array.from(file.name)
    .map((char) => (char.charCodeAt(0) < 32 || char.charCodeAt(0) === 127 ? "_" : char))
    .join("")
    .trim();
  const extension = contentTypeExtension(contentType);
  if (cleaned) {
    const truncated = Array.from(cleaned).slice(0, 255).join("");
    if (/\.[^./\\]+$/.test(truncated)) {
      return truncated;
    }
    return `${Array.from(truncated)
      .slice(0, 254 - extension.length)
      .join("")}.${extension}`;
  }
  const suffix = Math.random().toString(36).slice(2, 8);
  return `attachment-${Date.now()}-${suffix}.${extension}`;
}

export function attachmentRejection(
  file: Pick<File, "type" | "name" | "size">,
  currentCount: number,
): AttachmentRejection | null {
  if (currentCount >= MAX_ATTACHMENTS_PER_MESSAGE) {
    return "too-many";
  }
  if (file.size <= 0) {
    return "empty";
  }
  if (file.size > MAX_ATTACHMENT_FILE_SIZE) {
    return "too-large";
  }
  if (!resolveAttachmentContentType(file)) {
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
