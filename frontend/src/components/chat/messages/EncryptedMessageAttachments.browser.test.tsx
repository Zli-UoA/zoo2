import "../../../styles.css";
import { render, screen, waitFor } from "@testing-library/react";
import type { AttachmentInfo } from "@zoo/crypto-wasm";
import { expect, it, vi } from "vite-plus/test";

import { DEFAULT_CONFIG } from "../../../lib/config";
import { EncryptedAttachmentError } from "../../../lib/e2ee/attachments";
import type { AttachmentRow } from "../../../lib/electric/types";
import { LocaleProvider } from "../../i18n/LocaleProvider";
import {
  EncryptedAttachmentLoaderContext,
  EncryptedMessageAttachments,
} from "./EncryptedMessageAttachments";

vi.mock("../../../lib/electric/collections-context", () => ({
  useCollections: () => ({ config: DEFAULT_CONFIG }),
}));

const PNG = Uint8Array.from(
  atob(
    "iVBORw0KGgoAAAANSUhEUgAAAAMAAAABCAYAAAAb4BS0AAAADklEQVR4nGP438DwH4YBMWYHe7X3S+QAAAAASUVORK5CYII=",
  ),
  (c) => c.charCodeAt(0),
);

function attachment(): AttachmentInfo {
  // 行が再マウントされるたびに、復号した本文から同じ中身の新しいオブジェクトが渡される。
  return {
    attachment_id: "0b0d5c5e-6f2a-4c1e-9d7a-2f3b4c5d6e7f",
    name: "wide.png",
    mimetype: "image/png",
    size: PNG.byteLength,
    width: 3,
    height: 1,
    file: {
      v: "v2",
      key: { kty: "oct", key_ops: ["encrypt", "decrypt"], alg: "A256CTR", k: "key", ext: true },
      iv: "iv",
      hashes: { sha256: "hash" },
    },
  };
}

it("does not fetch and decrypt again when a virtualized row remounts", async () => {
  const loader = vi.fn(async (info: AttachmentInfo) => new Blob([PNG], { type: info.mimetype }));
  const element = () => (
    <LocaleProvider>
      <EncryptedAttachmentLoaderContext.Provider value={loader}>
        <EncryptedMessageAttachments attachments={[attachment()]} />
      </EncryptedAttachmentLoaderContext.Provider>
    </LocaleProvider>
  );

  const first = render(element());
  const image = (await screen.findByRole("img", { name: "wide.png" })) as HTMLImageElement;
  await waitFor(() => expect(image.naturalWidth).toBe(3));
  first.unmount();

  // 戻ってきた行は、スピナーを挟まずにキャッシュの画像を出す。
  render(element());
  const again = screen.getByRole("img", { name: "wide.png" }) as HTMLImageElement;
  await waitFor(() => expect(again.naturalWidth).toBe(3));
  expect(loader).toHaveBeenCalledTimes(1);
});

it("does not reuse decrypted attachments across crypto scopes (another worker or account)", async () => {
  const first = vi.fn(async (info: AttachmentInfo) => new Blob([PNG], { type: info.mimetype }));
  const second = vi.fn(async (info: AttachmentInfo) => new Blob([PNG], { type: info.mimetype }));
  const element = (loader: typeof first) => (
    <LocaleProvider>
      <EncryptedAttachmentLoaderContext.Provider value={loader}>
        <EncryptedMessageAttachments attachments={[attachment()]} />
      </EncryptedAttachmentLoaderContext.Provider>
    </LocaleProvider>
  );

  const view = render(element(first));
  await screen.findByRole("img", { name: "wide.png" });
  // ロックやログアウト、別のアカウントでは Worker が作り直され、キャッシュも別になる。
  view.rerender(element(second));
  await waitFor(() => expect(second).toHaveBeenCalledTimes(1));
  expect(first).toHaveBeenCalledTimes(1);
});

function view(
  loader: (info: AttachmentInfo) => Promise<Blob>,
  attachments: AttachmentInfo[],
  serverAttachments: AttachmentRow[] = [],
) {
  return (
    <LocaleProvider>
      <EncryptedAttachmentLoaderContext.Provider value={loader}>
        <EncryptedMessageAttachments
          attachments={attachments}
          serverAttachments={serverAttachments}
        />
      </EncryptedAttachmentLoaderContext.Provider>
    </LocaleProvider>
  );
}

it("blurs images the server marked sensitive for an NSFW channel", async () => {
  const loader = vi.fn(async (info: AttachmentInfo) => new Blob([PNG], { type: info.mimetype }));
  const info = attachment();
  // 本文には sensitive がないが、NSFW のチャンネルなのでサーバーの行に is_sensitive がある。
  render(view(loader, [info], [{ id: info.attachment_id, is_sensitive: true } as AttachmentRow]));
  expect(await screen.findByRole("button", { expanded: false })).toBeVisible();
  expect(screen.queryByRole("img", { name: "wide.png" })).toBeNull();
  expect(loader).not.toHaveBeenCalled();
});

it("does not fetch or offer to save file types the server would reject", async () => {
  const loader = vi.fn(async () => new Blob(["MZ"]));
  render(
    view(loader, [
      { ...attachment(), name: "請求書.pdf.exe", mimetype: "application/x-msdownload" },
    ]),
  );
  expect(await screen.findByText("請求書.pdf.exe")).toBeVisible();
  expect(screen.queryByRole("button", { name: /請求書/ })).toBeNull();
  expect(loader).not.toHaveBeenCalled();
});

it("does not fetch an undecryptable attachment again when the row remounts", async () => {
  const loader = vi.fn(async (): Promise<Blob> => {
    throw new EncryptedAttachmentError("decrypt");
  });
  const first = render(view(loader, [attachment()]));
  expect(await screen.findByRole("button", { name: /再試行|Retry/ })).toBeVisible();
  first.unmount();

  render(view(loader, [attachment()]));
  expect(screen.getByRole("button", { name: /再試行|Retry/ })).toBeVisible();
  expect(loader).toHaveBeenCalledTimes(1);
});

it("retries a remembered decryption failure when the download button is pressed", async () => {
  const loader = vi
    .fn<(info: AttachmentInfo) => Promise<Blob>>()
    .mockRejectedValueOnce(new EncryptedAttachmentError("decrypt"))
    .mockResolvedValue(new Blob(["%PDF-1.7"], { type: "application/pdf" }));
  const pdf = { ...attachment(), name: "minutes.pdf", mimetype: "application/pdf" };
  render(view(loader, [pdf]));
  const button = await screen.findByRole("button", { name: /minutes\.pdf/ });
  const click = vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(() => {});

  button.click();
  await waitFor(() => expect(loader).toHaveBeenCalledTimes(1));
  // 2 回目の押下は、覚えている失敗を使わずに取り直す。
  await waitFor(() => expect(button).not.toBeDisabled());
  button.click();
  await waitFor(() => expect(loader).toHaveBeenCalledTimes(2));
  await waitFor(() => expect(click).toHaveBeenCalledTimes(1));
  click.mockRestore();
});
