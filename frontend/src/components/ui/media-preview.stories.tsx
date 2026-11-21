import { expect, userEvent, waitFor, within } from "storybook/test";
import type { Meta, StoryObj } from "@storybook/react-vite";
import cjkUrl from "../../../.storybook/fixtures/cjk.pdf?url";
import jpxUrl from "../../../.storybook/fixtures/jpx.pdf?url";
import pdfUrl from "../../../.storybook/fixtures/media.pdf?url";
import audioUrl from "../../../.storybook/fixtures/media.wav?url";
import videoUrl from "../../../.storybook/fixtures/media.webm?url";
import emailUrl from "../../../.storybook/fixtures/sample.eml?url";
import msgUrl from "../../../.storybook/fixtures/sample.msg?url";
import docxUrl from "../../../.storybook/fixtures/sample.docx?url";
import xlsxUrl from "../../../.storybook/fixtures/sample.xlsx?url";
import pptxUrl from "../../../.storybook/fixtures/sample.pptx?url";
import odtUrl from "../../../.storybook/fixtures/sample.odt?url";
import zipUrl from "../../../.storybook/fixtures/sample.zip?url";
import { MediaPreview } from "./media-preview";

const meta = {
  title: "Components/Media/MediaPreview",
  component: MediaPreview,
  parameters: { a11y: { test: "error" } },
  args: { name: "sample.png", contentType: "image/png" },
} satisfies Meta<typeof MediaPreview>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Loading: Story = { args: { loading: true } };
export const Image: Story = {
  args: {
    src: "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAMAAAABCAYAAAAb4BS0AAAADklEQVR4nGP438DwH4YBMWYHe7X3S+QAAAAASUVORK5CYII=",
    width: 640,
    height: 480,
  },
};
export const PDF: Story = {
  args: { src: pdfUrl, name: "sample.pdf", contentType: "application/pdf" },
};
export const MultiPagePDF: Story = {
  args: { src: pdfUrl, name: "sample.pdf", contentType: "application/pdf", initialOpen: true },
  play: async ({ canvasElement }) => {
    await assertPdfPainted(canvasElement);
    const dialog = within(canvasElement.ownerDocument.body).getByRole("dialog");
    const user = userEvent.setup();
    const pageInput = within(dialog).getByRole("textbox", { name: "ページ番号" });
    const scrollRegion = within(dialog).getByRole("region", { name: "PDFページ一覧" });
    await expect(pageInput).toHaveValue("1");
    await user.click(within(dialog).getByRole("button", { name: "次のページ" }));
    await waitFor(() => expect(pageInput).toHaveValue("2"), { timeout: 10_000 });
    const page3 = dialog.querySelector('[data-pdf-page="3"]') as HTMLElement;
    scrollRegion.scrollTop = page3.offsetTop;
    await waitFor(() => expect(pageInput).toHaveValue("3"), { timeout: 10_000 });
    await user.click(pageInput);
    await user.keyboard("10{Enter}");
    await waitFor(
      async () => {
        await expect(pageInput).toHaveValue("10");
        await expect(
          (dialog.querySelector('[data-pdf-page="10"] canvas') as HTMLCanvasElement | null)?.width,
        ).toBeGreaterThan(0);
      },
      { timeout: 10_000 },
    );
  },
};
export const Video: Story = {
  args: { src: videoUrl, name: "sample.webm", contentType: "video/webm" },
};
export const Audio: Story = {
  args: { src: audioUrl, name: "sample.wav", contentType: "audio/wav" },
};
export const Failed: Story = { args: { failed: true, onRetry: () => {} } };
export const Narrow: Story = {
  ...Image,
  decorators: [
    (Story) => (
      <div style={{ width: 280 }}>
        <Story />
      </div>
    ),
  ],
};

export const Email: Story = {
  args: { src: emailUrl, name: "sample.eml", contentType: "message/rfc822", initialOpen: true },
  play: async ({ canvasElement }) => assertPreviewText(canvasElement, "Hello from email."),
};
export const Outlook: Story = {
  args: {
    src: msgUrl,
    name: "sample.msg",
    contentType: "application/vnd.ms-outlook",
    initialOpen: true,
  },
  play: async ({ canvasElement }) => assertPreviewText(canvasElement, "A memo."),
};
export const Word: Story = {
  args: {
    src: docxUrl,
    name: "sample.docx",
    contentType: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    initialOpen: true,
  },
  play: async ({ canvasElement }) => assertPreviewText(canvasElement, "Document preview"),
};
export const Spreadsheet: Story = {
  args: {
    src: xlsxUrl,
    name: "sample.xlsx",
    contentType: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    initialOpen: true,
  },
  play: async ({ canvasElement }) => assertPreviewText(canvasElement, "Example"),
};
export const Slides: Story = {
  args: {
    src: pptxUrl,
    name: "sample.pptx",
    contentType: "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    initialOpen: true,
  },
  play: async ({ canvasElement }) => assertPreviewText(canvasElement, "Slide preview"),
};
export const OpenDocument: Story = {
  args: {
    src: odtUrl,
    name: "sample.odt",
    contentType: "application/vnd.oasis.opendocument.text",
    initialOpen: true,
  },
  play: async ({ canvasElement }) => assertPreviewText(canvasElement, "OpenDocument preview"),
};
export const Archive: Story = {
  args: { src: zipUrl, name: "sample.zip", contentType: "application/zip", initialOpen: true },
};
export const Text: Story = {
  args: {
    src: "data:text/plain,Text%20preview",
    name: "sample.txt",
    contentType: "text/plain",
    initialOpen: true,
  },
  play: async ({ canvasElement }) => assertPreviewText(canvasElement, "Text preview"),
};

async function assertPreviewText(canvasElement: HTMLElement, expected: string) {
  const dialog = within(canvasElement.ownerDocument.body).getByRole("dialog");
  await waitFor(() => expect(dialog.querySelector("pre")?.textContent).toContain(expected), {
    timeout: 10_000,
  });
}

export const JapanesePDF: Story = {
  args: { src: cjkUrl, name: "cjk.pdf", contentType: "application/pdf", initialOpen: true },
  play: async ({ canvasElement }) => assertPdfPainted(canvasElement),
};
export const JPEG2000PDF: Story = {
  args: { src: jpxUrl, name: "jpx.pdf", contentType: "application/pdf", initialOpen: true },
  play: async ({ canvasElement }) => assertPdfPainted(canvasElement),
};
async function assertPdfPainted(canvasElement: HTMLElement) {
  const dialog = within(canvasElement.ownerDocument.body).getByRole("dialog");
  await waitFor(
    async () => {
      await expect(within(dialog).queryByRole("status")).toBeNull();
      await expect(within(dialog).queryByRole("alert")).toBeNull();
      await expect(dialog.querySelector("canvas")?.width).toBeGreaterThan(0);
    },
    { timeout: 10_000 },
  );
}

export const EmbeddedEmail: Story = {
  args: { src: emailUrl, name: "sample.eml", contentType: "message/rfc822", embedded: true },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(
      async () => {
        await expect(canvas.getByText("Hello from email.", { exact: false })).toBeVisible();
      },
      { timeout: 10_000 },
    );
    await expect(within(canvasElement.ownerDocument.body).queryByRole("dialog")).toBeNull();
  },
};
