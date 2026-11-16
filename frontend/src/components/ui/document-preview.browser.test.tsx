import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { expect, it } from "vite-plus/test";
import emailUrl from "../../../.storybook/fixtures/sample.eml?url";
import msgUrl from "../../../.storybook/fixtures/sample.msg?url";
import docxUrl from "../../../.storybook/fixtures/sample.docx?url";
import xlsxUrl from "../../../.storybook/fixtures/sample.xlsx?url";
import zipUrl from "../../../.storybook/fixtures/sample.zip?url";
import { LocaleProvider } from "../i18n/LocaleProvider";
import { MediaPreview } from "./media-preview";
import "../../styles.css";

it.each([
  ["sample.eml", "message/rfc822", emailUrl, "Hello from email."],
  ["sample.msg", "application/vnd.ms-outlook", msgUrl, "A memo."],
  [
    "sample.docx",
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    docxUrl,
    "Document preview",
  ],
  [
    "sample.xlsx",
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    xlsxUrl,
    "Example",
  ],
])("opens %s through the real worker only on request", async (name, contentType, src, expected) => {
  const view = render(
    <LocaleProvider>
      <MediaPreview src={src} name={name} contentType={contentType} />
    </LocaleProvider>,
  );
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(view.container.querySelector("pre")).toBeNull();
  fireEvent.click(screen.getByRole("button"));
  await waitFor(
    () => expect(screen.getByRole("dialog").querySelector("pre")?.textContent).toContain(expected),
    { timeout: 10_000 },
  );
  expect(document.querySelector('img[src*="tracking"]')).toBeNull();
  expect(document.querySelector("iframe")).toBeNull();
});
it("opens files inside a ZIP with the same viewer", async () => {
  render(
    <LocaleProvider>
      <MediaPreview src={zipUrl} name="sample.zip" contentType="application/zip" />
    </LocaleProvider>,
  );
  fireEvent.click(screen.getByRole("button"));
  const preview = await screen.findByRole(
    "button",
    { name: "readme.txtをプレビュー" },
    { timeout: 10_000 },
  );
  fireEvent.click(preview);
  await screen.findByText("Archive text", {}, { timeout: 10_000 });
  expect(screen.getAllByRole("dialog")).toHaveLength(1); // The nested modal makes the parent inert.
});
