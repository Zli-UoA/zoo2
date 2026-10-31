import { describe, expect, it } from "vite-plus/test";

import {
  ATTACHMENT_URL_FALLBACK_TTL_MS,
  MAX_ATTACHMENTS_PER_MESSAGE,
  MAX_ATTACHMENT_FILE_SIZE,
  attachmentFileName,
  attachmentFilesFromClipboard,
  attachmentRejection,
  attachmentUrlExpiresAt,
  formatAttachmentSize,
  groupAttachmentsByMessage,
  isInlineImage,
  resolveAttachmentContentType,
  shouldRefreshAttachmentUrl,
} from "./attachments";
import type { AttachmentRow } from "./electric/types";

function file(name: string, type: string, size = 10): Pick<File, "name" | "type" | "size"> {
  return { name, type, size };
}

function attachment(overrides: Partial<AttachmentRow> = {}): AttachmentRow {
  return {
    id: "attachment-1",
    message_id: "message-1",
    uploader_id: "user-1",
    channel_id: "channel-1",
    file_name: "image.png",
    file_size: "1024",
    content_type: "image/png",
    storage_key: "attachments/channel-1/attachment-1",
    created_at: "2026-01-01 00:00:00.000",
    ...overrides,
  };
}

describe("resolveAttachmentContentType", () => {
  it("accepts images and other files while rejecting executable formats", () => {
    expect(resolveAttachmentContentType(file("a.png", "image/png"))).toBe("image/png");
    expect(resolveAttachmentContentType(file("document.pdf", "application/pdf"))).toBe(
      "application/pdf",
    );
    expect(resolveAttachmentContentType(file("notes.txt", ""))).toBe("text/plain");
    expect(resolveAttachmentContentType(file("data.bin", ""))).toBe("application/octet-stream");
    expect(resolveAttachmentContentType(file("setup.exe", "application/x-msdownload"))).toBeNull();
    expect(resolveAttachmentContentType(file("run.sh", ""))).toBeNull();
  });
});

describe("attachmentFilesFromClipboard", () => {
  it("collects all files from items and files without duplicates", () => {
    const pasted = { name: "image.png", type: "image/png", size: 4, lastModified: 1 };
    const pdf = { name: "document.pdf", type: "application/pdf", size: 8, lastModified: 3 };
    const data = {
      files: [pasted],
      items: [
        { kind: "file", getAsFile: () => ({ ...pasted, lastModified: 2 }) },
        { kind: "string", getAsFile: () => null },
        { kind: "file", getAsFile: () => pdf },
      ],
    } as unknown as Pick<DataTransfer, "items" | "files">;

    expect(attachmentFilesFromClipboard(data)).toEqual([pasted, pdf]);
    expect(attachmentFilesFromClipboard(null)).toEqual([]);
  });
});

describe("attachment URL expiry", () => {
  it("derives expiry from the presigned GET URL", () => {
    const url =
      "https://attachments.zliusercontent.com/object?X-Amz-Date=20261031T120000Z&X-Amz-Expires=300&X-Amz-Signature=test";
    expect(attachmentUrlExpiresAt(url)).toBe(Date.parse("2026-10-31T12:00:00Z") + 300_000);
  });

  it("falls back to a conservative TTL when the URL has no expiry", () => {
    const now = 1_000_000;
    expect(attachmentUrlExpiresAt("data:image/png;base64,AAAA", now)).toBe(
      now + ATTACHMENT_URL_FALLBACK_TTL_MS,
    );
  });

  it("refreshes missing or nearly expired URLs before opening them", () => {
    const now = 1_000_000;
    expect(shouldRefreshAttachmentUrl(null, now)).toBe(true);
    expect(shouldRefreshAttachmentUrl(now + 1_000, now)).toBe(true);
    expect(shouldRefreshAttachmentUrl(now + 60_000, now)).toBe(false);
  });
});

describe("attachment validation", () => {
  it("rejects too many, empty, oversized, and executable files", () => {
    expect(attachmentRejection(file("a.png", "image/png"), MAX_ATTACHMENTS_PER_MESSAGE)).toBe(
      "too-many",
    );
    expect(attachmentRejection(file("a.png", "image/png", 0), 0)).toBe("empty");
    expect(attachmentRejection(file("a.png", "image/png", MAX_ATTACHMENT_FILE_SIZE + 1), 0)).toBe(
      "too-large",
    );
    expect(attachmentRejection(file("setup.exe", "application/x-msdownload"), 0)).toBe(
      "unsupported",
    );
    expect(attachmentRejection(file("document.pdf", "application/pdf"), 0)).toBeNull();
    expect(attachmentRejection(file("a.png", "image/png"), 0)).toBeNull();
  });

  it("sanitizes pasted file names and supplies a fallback extension", () => {
    expect(attachmentFileName({ name: "bad\nname.png" }, "image/png")).toBe("bad_name.png");
    expect(attachmentFileName({ name: "release-notes" }, "application/pdf")).toBe(
      "release-notes.pdf",
    );
    const generated = attachmentFileName({ name: "" }, "image/jpeg");
    expect(generated).toMatch(/^attachment-\d+-[a-z0-9]+\.jpg$/);
  });
});

describe("attachment presentation", () => {
  it("formats byte sizes for the active locale", () => {
    expect(formatAttachmentSize("1024", "ja")).toBe("1 KB");
    expect(formatAttachmentSize(1536, "en")).toBe("1.5 KB");
    expect(formatAttachmentSize("invalid")).toBe("—");
  });

  it("detects inline images and groups attachments by message in creation order", () => {
    expect(isInlineImage("image/png; charset=binary")).toBe(true);
    expect(isInlineImage("application/pdf")).toBe(false);

    const later = attachment({ id: "later", created_at: "2026-01-01 00:00:02.000" });
    const earlier = attachment({ id: "earlier", created_at: "2026-01-01 00:00:01.000" });
    const other = attachment({ id: "other", message_id: "message-2" });
    const grouped = groupAttachmentsByMessage([later, other, earlier]);
    expect(grouped.get("message-1")?.map((row) => row.id)).toEqual(["earlier", "later"]);
    expect(grouped.get("message-2")?.map((row) => row.id)).toEqual(["other"]);
    expect(groupAttachmentsByMessage([attachment({ message_id: null })])).toEqual(new Map());
  });
});
