import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vite-plus/test";

import { LocaleProvider } from "../../i18n/LocaleProvider";
import { Composer } from "./Composer";

function imageFile(): File {
  return new File([new Uint8Array([1, 2, 3, 4])], "screenshot.png", { type: "image/png" });
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
    const clipboardData = new DataTransfer();
    clipboardData.items.add(file);
    screen
      .getByRole("textbox")
      .dispatchEvent(
        new ClipboardEvent("paste", { clipboardData, bubbles: true, cancelable: true }),
      );

    await waitFor(() => expect(upload).toHaveBeenCalledWith(file));
    const sendButton = screen.getByRole("button", { name: "送信" });
    await waitFor(() => expect(sendButton).toBeEnabled());
    await userEvent.click(sendButton);

    await waitFor(() => expect(send).toHaveBeenCalledWith("", undefined, ["attachment-1"]));
    expect(screen.queryByRole("button", { name: "screenshot.pngを削除" })).toBeNull();
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

    const clipboardData = new DataTransfer();
    clipboardData.items.add(imageFile());
    screen
      .getByRole("textbox")
      .dispatchEvent(
        new ClipboardEvent("paste", { clipboardData, bubbles: true, cancelable: true }),
      );

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "画像をアップロードできませんでした。",
    );
    expect(screen.queryByRole("button", { name: "screenshot.pngを削除" })).toBeNull();
  });
});
