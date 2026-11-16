import PostalMime, { type Address } from "postal-mime";
import MsgReader from "@kenjiuno/msgreader";
import { unzipSync } from "fflate";
import { SaxesParser } from "saxes";
import { resolveAttachmentContentType } from "../attachments";
import {
  MAX_EXPANDED_BYTES,
  MAX_PREVIEW_BYTES,
  MAX_PREVIEW_FILES,
  MAX_PREVIEW_TEXT,
  type DocumentPreviewData,
  type PreviewFile,
} from "./types";

import { PreviewLimitError } from "./fetch";
export { PreviewLimitError } from "./fetch";

function text(bytes: Uint8Array, type = "") {
  const charset = /charset\s*=\s*["']?([^\s;"']+)/i.exec(type)?.[1] ?? "utf-8";
  try {
    return new TextDecoder(charset).decode(bytes);
  } catch {
    return new TextDecoder().decode(bytes);
  }
}
function section(name: string, value: string) {
  if (value.length > MAX_PREVIEW_TEXT) throw new PreviewLimitError();
  return { name, text: value };
}
/** Convert HTML mail to inert text. Never render mail HTML or load remote resources. */
function htmlText(value: string) {
  return value
    .replace(/<(script|style)\b[^>]*>[\s\S]*?<\/\1\s*>/gi, "")
    .replace(/<(?:br|\/p|\/div|\/tr|\/h[1-6])\b[^>]*>/gi, "\n")
    .replace(/<[^>]*>/g, "")
    .replace(/&(#x[\da-f]+|#\d+|amp|lt|gt|quot|apos|nbsp);/gi, (_, entity: string) => {
      const named: Record<string, string> = {
        amp: "&",
        lt: "<",
        gt: ">",
        quot: '"',
        apos: "'",
        nbsp: " ",
      };
      if (!entity.startsWith("#")) return named[entity.toLowerCase()] ?? "";
      const code =
        entity[1].toLowerCase() === "x" ? parseInt(entity.slice(2), 16) : Number(entity.slice(1));
      return code > 0 && code <= 0x10ffff ? String.fromCodePoint(code) : "";
    })
    .trim();
}
function address(value: Address): string {
  return value.group
    ? `${value.name}: ${value.group.map(address).join(", ")}`
    : [value.name, value.address].filter(Boolean).join(" ");
}
function validateFiles(files: PreviewFile[]) {
  if (
    files.length > MAX_PREVIEW_FILES ||
    files.reduce((total, file) => total + file.bytes.byteLength, 0) > MAX_EXPANDED_BYTES
  )
    throw new PreviewLimitError();
  return files;
}
function archive(bytes: Uint8Array) {
  let count = 0,
    total = 0;
  const entries = unzipSync(bytes, {
    filter: (entry) => {
      if (++count > MAX_PREVIEW_FILES || (total += entry.originalSize) > MAX_EXPANDED_BYTES)
        throw new PreviewLimitError();
      return !entry.name.endsWith("/");
    },
  });
  return entries;
}
function xmlChunks(xml: Uint8Array, tag: string): string[] {
  const values: string[] = [];
  const parser = new SaxesParser({ xmlns: true });
  let capture = 0,
    current = "";
  parser.on("doctype", () => {
    throw new Error("DTD is not supported");
  });
  parser.on("opentag", (node) => {
    if (node.local === tag) {
      if (!capture) current = "";
      capture++;
    }
  });
  parser.on("text", (value) => {
    if (capture) current += value;
  });
  parser.on("cdata", (value) => {
    if (capture) current += value;
  });
  parser.on("closetag", (node) => {
    if (node.local === tag && --capture === 0) values.push(current);
  });
  parser.write(text(xml)).close();
  return values;
}
function sheetText(xml: Uint8Array, shared: string[]) {
  const parser = new SaxesParser({ xmlns: true });
  const rows: string[] = [];
  let cells: string[] = [],
    value = "",
    capture = false,
    sharedCell = false;
  parser.on("doctype", () => {
    throw new Error("DTD is not supported");
  });
  parser.on("opentag", (node) => {
    if (node.local === "c") {
      const ref = Object.values(node.attributes).find((a) => a.local === "r")?.value;
      const letters = ref?.match(/^[A-Z]+/i)?.[0];
      if (letters) {
        const column =
          letters
            .toUpperCase()
            .split("")
            .reduce((n, char) => n * 26 + char.charCodeAt(0) - 64, 0) - 1;
        if (column > 1024) throw new PreviewLimitError();
        while (cells.length < column) cells.push("");
      }
      value = "";
      sharedCell = Object.values(node.attributes).some((a) => a.local === "t" && a.value === "s");
    }
    if (node.local === "v" || node.local === "t") capture = true;
  });
  parser.on("text", (content) => {
    if (capture) value += content;
  });
  parser.on("closetag", (node) => {
    if (node.local === "v" || node.local === "t") capture = false;
    if (node.local === "c") cells.push(sharedCell ? (shared[Number(value)] ?? value) : value);
    if (node.local === "row") {
      rows.push(cells.join("\t"));
      cells = [];
    }
  });
  parser.write(text(xml)).close();
  return rows.join("\n");
}
export async function parseDocument(
  bytes: Uint8Array<ArrayBuffer>,
  name: string,
  contentType: string,
): Promise<DocumentPreviewData> {
  if (bytes.byteLength > MAX_PREVIEW_BYTES) throw new PreviewLimitError();
  const mime = contentType.split(";")[0].trim().toLowerCase();
  const ext = name.split(".").pop()?.toLowerCase();
  const empty = { sections: [], files: [] };
  if (mime === "message/rfc822" || ext === "eml") {
    const mail = await PostalMime.parse(bytes, { maxRfc822NestingDepth: 4 });
    return {
      headers: [
        { label: "subject", value: mail.subject ?? "" },
        { label: "from", value: mail.from ? address(mail.from) : "" },
        { label: "to", value: mail.to?.map(address).join(", ") ?? "" },
        { label: "cc", value: mail.cc?.map(address).join(", ") ?? "" },
        { label: "date", value: mail.date ?? "" },
      ],
      sections: [section(name, mail.text ?? htmlText(mail.html ?? ""))],
      files: validateFiles(
        mail.attachments.map((file, index) => ({
          name: file.filename ?? `attachment-${index + 1}`,
          type: file.mimeType,
          bytes:
            typeof file.content === "string"
              ? new TextEncoder().encode(file.content)
              : new Uint8Array(file.content),
        })),
      ),
    };
  }
  if (mime === "application/vnd.ms-outlook" || ext === "msg") {
    // The package is CommonJS; Node and browser bundles expose different default wrappers.
    const Reader =
      typeof MsgReader === "function"
        ? MsgReader
        : (MsgReader as unknown as { default: typeof MsgReader }).default;
    const reader = new Reader(bytes.buffer);
    const mail = reader.getFileData();
    if (mail.error) throw new Error(mail.error);
    return {
      headers: [
        { label: "subject", value: mail.subject ?? "" },
        { label: "from", value: [mail.senderName, mail.senderEmail].filter(Boolean).join(" ") },
        {
          label: "to",
          value:
            mail.recipients
              ?.filter((r) => r.recipType !== "cc" && r.recipType !== "bcc")
              .map((r) => r.email ?? r.name ?? "")
              .join(", ") ?? "",
        },
        { label: "date", value: mail.creationTime ?? "" },
      ],
      sections: [section(name, mail.body ?? htmlText(mail.bodyHtml ?? ""))],
      files: validateFiles(
        (mail.attachments ?? []).map((file) => {
          const attachment = reader.getAttachment(file);
          return {
            name: attachment.fileName,
            type:
              file.attachMimeTag ??
              resolveAttachmentContentType({ name: attachment.fileName, type: "" }) ??
              "application/octet-stream",
            bytes: new Uint8Array(attachment.content),
          };
        }),
      ),
    };
  }
  if (
    mime === "application/zip" ||
    mime === "application/x-zip-compressed" ||
    ["zip", "docx", "xlsx", "pptx", "odt", "ods", "odp"].includes(ext ?? "") ||
    mime.includes("officedocument") ||
    mime.includes("opendocument")
  ) {
    const entries = archive(bytes);
    const sections: DocumentPreviewData["sections"] = [];
    const shared = entries["xl/sharedStrings.xml"]
      ? xmlChunks(entries["xl/sharedStrings.xml"], "si")
      : [];
    for (const path of Object.keys(entries).sort((a, b) =>
      a.localeCompare(b, "en", { numeric: true }),
    )) {
      if (path === "word/document.xml")
        sections.push(section(name, xmlChunks(entries[path], "p").join("\n")));
      if (/^ppt\/slides\/slide\d+\.xml$/.test(path))
        sections.push(section(path, xmlChunks(entries[path], "p").join("\n")));
      if (/^xl\/worksheets\/sheet\d+\.xml$/.test(path))
        sections.push(section(path, sheetText(entries[path], shared)));
      if (path === "content.xml" && ["odt", "ods", "odp"].includes(ext ?? ""))
        sections.push(section(name, xmlChunks(entries[path], "p").join("\n")));
    }
    if (
      mime.includes("officedocument") ||
      mime.includes("opendocument") ||
      ["docx", "xlsx", "pptx", "odt", "ods", "odp"].includes(ext ?? "")
    ) {
      if (!sections.length) throw new Error("Missing document contents");
      if (sections.reduce((n, item) => n + item.text.length, 0) > MAX_PREVIEW_TEXT)
        throw new PreviewLimitError();
      return { sections, files: [] };
    }
    return {
      ...empty,
      files: validateFiles(
        Object.entries(entries).map(([path, data]) => ({
          name: path,
          type:
            resolveAttachmentContentType({ name: path, type: "" }) ?? "application/octet-stream",
          bytes: new Uint8Array(data),
        })),
      ),
    };
  }
  let value = text(bytes, contentType);
  if (mime === "application/json" || ext === "json") {
    try {
      value = JSON.stringify(JSON.parse(value), null, 2);
    } catch {
      /* Invalid JSON remains readable as text. */
    }
  }
  return { ...empty, sections: [section(name, value)] };
}
