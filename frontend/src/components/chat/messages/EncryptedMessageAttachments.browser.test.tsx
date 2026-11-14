import "../../../styles.css";
import { render, screen, waitFor } from "@testing-library/react";
import type { AttachmentInfo } from "@zoo/crypto-wasm";
import { expect, it, vi } from "vite-plus/test";

import { DEFAULT_CONFIG } from "../../../lib/config";
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
