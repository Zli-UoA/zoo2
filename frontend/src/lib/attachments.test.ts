import { describe, expect, it } from "vite-plus/test";

import {
  MAX_ATTACHMENTS_PER_MESSAGE,
  MAX_ATTACHMENT_FILE_SIZE,
  attachmentFileName,
  attachmentRejection,
  formatAttachmentSize,
  groupAttachmentsByMessage,
  imageFilesFromClipboard,
  isInlineImage,
  resolveSupportedImageType,
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

describe("resolveSupportedImageType", () => {
  it("accepts the inline image allowlist", () => {
    expect(resolveSupportedImageType(file("a.png", "image/png"))).toBe("image/png");
    expect(resolveSupportedImageType(file("a.jpg", "image/jpeg"))).toBe("image/jpeg");
    expect(resolveSupportedImageType(file("a.webp", "image/webp"))).toBe("image/webp");
    expect(resolveSupportedImageType(file("a.gif", "image/gif"))).toBe("image/gif");
  });

  it("falls back to the extension and rejects other formats", () => {
    expect(resolveSupportedImageType(file("screenshot.PNG", ""))).toBe("image/png");
    expect(resolveSupportedImageType(file("photo.jpeg", ""))).toBe("image/jpeg");
    expect(resolveSupportedImageType(file("diagram.svg", "image/svg+xml"))).toBeNull();
    expect(resolveSupportedImageType(file("archive.zip", "application/zip"))).toBeNull();
  });
});

describe("imageFilesFromClipboard", () => {
  it("collects image files from items and files without duplicates", () => {
    const pasted = { name: "image.png", type: "image/png", size: 4, lastModified: 1 };
    const data = {
      files: [pasted],
      items: [
        { kind: "file", getAsFile: () => pasted },
        { kind: "string", getAsFile: () => null },
        { kind: "file", getAsFile: () => ({ ...pasted, name: "notes.txt", type: "text/plain" }) },
      ],
    } as unknown as Pick<DataTransfer, "items" | "files">;

    expect(imageFilesFromClipboard(data)).toEqual([pasted]);
    expect(imageFilesFromClipboard(null)).toEqual([]);
  });
});

describe("attachment validation", () => {
  it("rejects too many, oversized, and unsupported files", () => {
    expect(attachmentRejection(file("a.png", "image/png"), MAX_ATTACHMENTS_PER_MESSAGE)).toBe(
      "too-many",
    );
    expect(attachmentRejection(file("a.png", "image/png", MAX_ATTACHMENT_FILE_SIZE + 1), 0)).toBe(
      "too-large",
    );
    expect(attachmentRejection(file("a.svg", "image/svg+xml"), 0)).toBe("unsupported");
    expect(attachmentRejection(file("a.png", "image/png"), 0)).toBeNull();
  });

  it("sanitizes pasted file names and supplies a fallback", () => {
    expect(attachmentFileName({ name: "bad\nname.png" }, "image/png")).toBe("bad_name.png");
    const generated = attachmentFileName({ name: "" }, "image/jpeg");
    expect(generated).toMatch(/^pasted-image-\d+-[a-z0-9]+\.jpg$/);
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
