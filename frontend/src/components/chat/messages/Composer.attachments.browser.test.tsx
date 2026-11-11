import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vite-plus/test";

import { LocaleProvider } from "../../i18n/LocaleProvider";
import { Composer } from "./Composer";

function imageFile(): File {
  return new File([new Uint8Array([1, 2, 3, 4])], "screenshot.png", { type: "image/png" });
}

function pdfFile(): File {
  return new File([new Uint8Array([1, 2, 3, 4, 5])], "release-notes.pdf", {
    type: "application/pdf",
  });
}

function paste(target: HTMLElement, file: File) {
  const clipboardData = new DataTransfer();
  clipboardData.items.add(file);
  target.dispatchEvent(
    new ClipboardEvent("paste", { clipboardData, bubbles: true, cancelable: true }),
  );
}

describe("Composer attachments", () => {
  it("uploads a pasted image and sends the attachment id with the message", async () => {
    const upload = vi.fn(async () => ({
      id: "attachment-1",
      file_name: "screenshot.png",
      file_size: 4,
      content_type: "image/png",
    }));
    const send = vi.fn(async () => true);
    render(
      <LocaleProvider>
        <Composer onSend={send} onUploadAttachment={upload} />
      </LocaleProvider>,
    );

    const file = imageFile();
    paste(screen.getByRole("textbox"), file);

    await waitFor(() => expect(upload).toHaveBeenCalledWith(file));
    const sendButton = screen.getByRole("button", { name: "送信" });
    await waitFor(() => expect(sendButton).toBeEnabled());
    await userEvent.click(sendButton);

    await waitFor(() => expect(send).toHaveBeenCalledWith("", undefined, ["attachment-1"]));
    expect(screen.queryByRole("button", { name: "screenshot.pngを削除" })).toBeNull();
  });

  it("sends the sensitive label with a selected image", async () => {
    const upload = vi.fn(async () => ({
      id: "sensitive-1",
      file_name: "screenshot.png",
      file_size: 4,
      content_type: "image/png",
    }));
    const send = vi.fn(async () => true);
    render(
      <LocaleProvider>
        <Composer onSend={send} onUploadAttachment={upload} />
      </LocaleProvider>,
    );
    paste(screen.getByRole("textbox"), imageFile());
    await waitFor(() => expect(screen.getByRole("button", { name: "送信" })).toBeEnabled());
    await userEvent.click(screen.getByRole("checkbox", { name: "screenshot.png: センシティブ" }));
    await userEvent.click(screen.getByRole("button", { name: "送信" }));
    await waitFor(() =>
      expect(send).toHaveBeenCalledWith("", undefined, ["sensitive-1"], ["sensitive-1"]),
    );
    paste(screen.getByRole("textbox"), imageFile());
    await waitFor(() => expect(screen.getByRole("button", { name: "送信" })).toBeEnabled());
    expect(
      screen.getByRole("checkbox", { name: "screenshot.png: センシティブ" }),
    ).not.toBeChecked();
    await userEvent.click(screen.getByRole("button", { name: "送信" }));
    await waitFor(() => expect(send).toHaveBeenLastCalledWith("", undefined, ["sensitive-1"]));
  });

  it("starts a new image draft without inheriting an in-flight send's label", async () => {
    const upload = vi.fn(async () => ({
      id: "pending-1",
      file_name: "screenshot.png",
      file_size: 4,
      content_type: "image/png",
    }));
    let complete!: (sent: boolean) => void;
    const send = vi.fn(
      () =>
        new Promise<boolean>((resolve) => {
          complete = resolve;
        }),
    );
    render(
      <LocaleProvider>
        <Composer onSend={send} onUploadAttachment={upload} />
      </LocaleProvider>,
    );
    paste(screen.getByRole("textbox"), imageFile());
    await waitFor(() => expect(screen.getByRole("button", { name: "送信" })).toBeEnabled());
    await userEvent.click(screen.getByRole("checkbox", { name: "screenshot.png: センシティブ" }));
    await userEvent.click(screen.getByRole("button", { name: "送信" }));
    await waitFor(() =>
      expect(send).toHaveBeenCalledWith("", undefined, ["pending-1"], ["pending-1"]),
    );
    paste(screen.getByRole("textbox"), imageFile());
    await waitFor(() => expect(screen.getByRole("button", { name: "送信" })).toBeEnabled());
    expect(
      screen.getByRole("checkbox", { name: "screenshot.png: センシティブ" }),
    ).not.toBeChecked();
    complete(true);
    expect(
      screen.getByRole("checkbox", { name: "screenshot.png: センシティブ" }),
    ).not.toBeChecked();
  });

  it("preserves the sensitive label after failure so the draft can be retried", async () => {
    const upload = vi.fn(async () => ({
      id: "retry-1",
      file_name: "screenshot.png",
      file_size: 4,
      content_type: "image/png",
    }));
    const send = vi.fn().mockResolvedValueOnce(false).mockResolvedValueOnce(true);
    render(
      <LocaleProvider>
        <Composer onSend={send} onUploadAttachment={upload} />
      </LocaleProvider>,
    );
    paste(screen.getByRole("textbox"), imageFile());
    await waitFor(() => expect(screen.getByRole("button", { name: "送信" })).toBeEnabled());
    await userEvent.click(screen.getByRole("checkbox", { name: "screenshot.png: センシティブ" }));
    await userEvent.click(screen.getByRole("button", { name: "送信" }));
    await waitFor(() =>
      expect(screen.getByRole("checkbox", { name: "screenshot.png: センシティブ" })).toBeChecked(),
    );
    await userEvent.click(screen.getByRole("button", { name: "送信" }));
    await waitFor(() =>
      expect(send).toHaveBeenNthCalledWith(2, "", undefined, ["retry-1"], ["retry-1"]),
    );
  });

  it("marks only the selected image when two images are attached", async () => {
    const upload = vi.fn(async (file: File) => ({
      id: file.name,
      file_name: file.name,
      file_size: file.size,
      content_type: file.type,
    }));
    const send = vi.fn(async () => true);
    render(
      <LocaleProvider>
        <Composer onSend={send} onUploadAttachment={upload} />
      </LocaleProvider>,
    );
    for (const name of ["first.png", "second.png"]) {
      paste(
        screen.getByRole("textbox"),
        new File([new Uint8Array([1, 2, 3, 4])], name, { type: "image/png" }),
      );
      await waitFor(() => expect(screen.getByRole("button", { name: "送信" })).toBeEnabled());
    }
    await userEvent.click(screen.getByRole("checkbox", { name: "second.png: センシティブ" }));
    expect(screen.getByRole("checkbox", { name: "first.png: センシティブ" })).not.toBeChecked();
    await userEvent.click(screen.getByRole("button", { name: "送信" }));
    await waitFor(() =>
      expect(send).toHaveBeenCalledWith("", undefined, ["first.png", "second.png"], ["second.png"]),
    );
  });

  it("uploads a pasted non-image file and sends its attachment id", async () => {
    const upload = vi.fn(async () => ({
      id: "attachment-2",
      file_name: "release-notes.pdf",
      file_size: 5,
      content_type: "application/pdf",
    }));
    const send = vi.fn(async () => true);
    render(
      <LocaleProvider>
        <Composer onSend={send} onUploadAttachment={upload} />
      </LocaleProvider>,
    );

    const file = pdfFile();
    paste(screen.getByRole("textbox"), file);

    await waitFor(() => expect(upload).toHaveBeenCalledWith(file));
    expect(await screen.findByText("release-notes.pdf")).toBeVisible();
    const sendButton = screen.getByRole("button", { name: "送信" });
    await waitFor(() => expect(sendButton).toBeEnabled());
    await userEvent.click(sendButton);

    await waitFor(() => expect(send).toHaveBeenCalledWith("", undefined, ["attachment-2"]));
  });

  it("shows an error and removes the preview when upload fails", async () => {
    const upload = vi.fn(async () => {
      throw new Error("upload failed");
    });
    render(
      <LocaleProvider>
        <Composer onSend={async () => true} onUploadAttachment={upload} />
      </LocaleProvider>,
    );

    paste(screen.getByRole("textbox"), imageFile());

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "ファイルをアップロードできませんでした。",
    );
    expect(screen.queryByRole("button", { name: "screenshot.pngを削除" })).toBeNull();
  });
});
