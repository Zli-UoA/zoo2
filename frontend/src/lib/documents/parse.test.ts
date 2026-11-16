import { readFile } from "node:fs/promises";
import { expect, it } from "vite-plus/test";
import { zipSync, strToU8 } from "fflate";
import { parseDocument, PreviewLimitError } from "./parse";
import { MAX_EXPANDED_BYTES } from "./types";
const fixture = async (name: string) =>
  new Uint8Array(await readFile(new URL(`../../../.storybook/fixtures/${name}`, import.meta.url)));

it("decodes MIME headers and attachments without including HTML scripts or tracking images", async () => {
  const mail = await parseDocument(await fixture("sample.eml"), "sample.eml", "message/rfc822");
  expect(mail.headers?.find((h) => h.label === "subject")?.value).toBe("プレビュー確認");
  expect(mail.headers?.find((h) => h.label === "from")?.value).toContain("sender@example.test");
  expect(mail.sections[0].text).toContain("Hello from email.");
  expect(mail.sections[0].text).not.toMatch(/script|tracking|alert/);
  expect(mail.files[0].name).toBe("note.txt");
  expect(new TextDecoder().decode(mail.files[0].bytes)).toBe("添付ファイルの本文");
});
it("reads Outlook MSG", async () => {
  const mail = await parseDocument(
    await fixture("sample.msg"),
    "sample.msg",
    "application/vnd.ms-outlook",
  );
  expect(mail.headers?.find((h) => h.label === "subject")?.value).toBe("A memo.");
  expect(mail.sections[0].text).toContain("A memo.");
});
it.each([
  ["sample.docx", "Document preview"],
  ["sample.xlsx", "Item\t金額\nExample\t123"],
  ["sample.pptx", "Slide preview"],
  ["sample.odt", "OpenDocument preview"],
])("extracts text/cells from %s", async (name, expected) => {
  const document = await parseDocument(await fixture(name), name, "application/octet-stream");
  expect(document.sections[0].text).toContain(expected);
  expect(document.files).toHaveLength(0);
});
it("lists ZIP files and preserves bytes and inferred content types", async () => {
  const result = await parseDocument(await fixture("sample.zip"), "sample.zip", "application/zip");
  expect(result.files.map((f) => f.name)).toEqual(["readme.txt", "folder/data.csv"]);
  expect(result.files[1].type).toBe("text/csv");
  expect(new TextDecoder().decode(result.files[0].bytes)).toBe("Archive text");
});
it("rejects expanded archives, excessive entries, malformed documents and DTDs", async () => {
  await expect(
    parseDocument(
      zipSync({ large: new Uint8Array(MAX_EXPANDED_BYTES + 1) }),
      "large.zip",
      "application/zip",
    ),
  ).rejects.toBeInstanceOf(PreviewLimitError);
  const many = Object.fromEntries(
    Array.from({ length: 101 }, (_, i) => [String(i), strToU8("test")]),
  );
  await expect(parseDocument(zipSync(many), "many.zip", "application/zip")).rejects.toBeInstanceOf(
    PreviewLimitError,
  );
  await expect(
    parseDocument(strToU8("broken"), "bad.docx", "application/octet-stream"),
  ).rejects.toThrow();
  await expect(
    parseDocument(
      zipSync({
        "word/document.xml": strToU8(
          '<!DOCTYPE x [<!ENTITY a SYSTEM "https://external.test/">]><p>&a;</p>',
        ),
      }),
      "x.docx",
      "application/octet-stream",
    ),
  ).rejects.toThrow();
});
it("formats JSON, preserves invalid JSON and honors declared text encoding", async () => {
  expect(
    (await parseDocument(strToU8('{"a":1}'), "x.json", "application/json")).sections[0].text,
  ).toContain('  "a": 1');
  expect(
    (await parseDocument(strToU8("{broken"), "x.json", "application/json")).sections[0].text,
  ).toBe("{broken");
  expect(
    (await parseDocument(new Uint8Array([0x82, 0xa0]), "x.txt", "text/plain; charset=shift_jis"))
      .sections[0].text,
  ).toBe("あ");
});
