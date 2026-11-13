import { readFileSync } from "node:fs";

import { decryptAttachment, encryptAttachment, initSync } from "@zoo/crypto-wasm";
import { beforeAll, describe, expect, it, vi } from "vite-plus/test";

import { AttachmentUploadError } from "../api/attachments";
import type { ApiClient } from "../api/client";
import {
  ENCRYPTED_ATTACHMENT_CONTENT_TYPE,
  ENCRYPTED_ATTACHMENT_FILE_NAME,
  EncryptedAttachmentError,
  fetchEncryptedAttachment,
  uploadEncryptedAttachment,
  type AttachmentCrypto,
} from "./attachments";

beforeAll(() => {
  const wasm = readFileSync(
    new URL("../../../../crates/zoo-crypto-wasm/pkg/zoo_crypto_wasm_bg.wasm", import.meta.url),
  );
  initSync({ module: wasm });
});

// Worker を通さず、同じ wasm の関数で暗号化と復号をする。
const wasmCrypto: AttachmentCrypto = {
  encrypt: async (plaintext) => encryptAttachment(plaintext),
  decrypt: async (file, ciphertext) => decryptAttachment(file, ciphertext),
};

const MARKER = "ZOO2-E2EE-364-PLAINTEXT";
const PLAINTEXT = new TextEncoder().encode(`${MARKER} ${MARKER} ${MARKER}`);

function file(name = "secret.txt", type = "text/plain"): File {
  return new File([PLAINTEXT], name, { type });
}

function contains(haystack: Uint8Array, needle: Uint8Array): boolean {
  outer: for (let i = 0; i + needle.length <= haystack.length; i++) {
    for (let j = 0; j < needle.length; j++) {
      if (haystack[i + j] !== needle[j]) continue outer;
    }
    return true;
  }
  return false;
}

function presignClient() {
  const post = vi.fn(async () => ({
    data: {
      attachment_id: "0b0d5c5e-6f2a-4c1e-9d7a-2f3b4c5d6e7f",
      upload_url: "https://storage.example/upload",
      storage_key: "attachments/channel-1/0b0d5c5e-6f2a-4c1e-9d7a-2f3b4c5d6e7f",
    },
    txid: null,
  }));
  return { post, client: { post } as unknown as ApiClient };
}

async function uploaded(fetchImpl: ReturnType<typeof vi.fn>): Promise<Uint8Array> {
  const init = fetchImpl.mock.calls[0]![1] as RequestInit;
  return new Uint8Array(await (init.body as Blob).arrayBuffer());
}

describe("uploadEncryptedAttachment", () => {
  it("uploads only the ciphertext with the fixed name and content type", async () => {
    const { post, client } = presignClient();
    const fetchImpl = vi.fn(async () => new Response(null, { status: 200 }));

    const info = await uploadEncryptedAttachment(client, "channel-1", file(), wasmCrypto, {
      fetchImpl: fetchImpl as typeof fetch,
    });

    // サーバーに渡すのは決まったファイル名と Content-Type、暗号文の大きさだけ。
    expect(post).toHaveBeenCalledWith("/api/attachments/presign", {
      channel_id: "channel-1",
      file_name: ENCRYPTED_ATTACHMENT_FILE_NAME,
      file_size: PLAINTEXT.byteLength,
      content_type: ENCRYPTED_ATTACHMENT_CONTENT_TYPE,
    });
    expect(fetchImpl).toHaveBeenCalledWith(
      "https://storage.example/upload",
      expect.objectContaining({
        method: "PUT",
        headers: { "Content-Type": ENCRYPTED_ATTACHMENT_CONTENT_TYPE },
      }),
    );
    // S3 に上げるのは暗号文で、平文の並びは残らない。
    const body = await uploaded(fetchImpl);
    expect(body.byteLength).toBe(PLAINTEXT.byteLength);
    expect(contains(body, new TextEncoder().encode(MARKER))).toBe(false);
    expect(decryptAttachment(info.file, body)).toEqual(PLAINTEXT);

    // 本当の名前と MIME は、本文に入れる AttachmentInfo にだけある。
    expect(info).toMatchObject({
      attachment_id: "0b0d5c5e-6f2a-4c1e-9d7a-2f3b4c5d6e7f",
      name: "secret.txt",
      mimetype: "text/plain",
      size: PLAINTEXT.byteLength,
    });
    expect(info.width).toBeUndefined();
    expect(info.sensitive).toBeUndefined();
  });

  it("records the image size and the sensitive flag in the payload", async () => {
    const { client } = presignClient();
    const info = await uploadEncryptedAttachment(
      client,
      "channel-1",
      file("photo.png", "image/png"),
      wasmCrypto,
      {
        fetchImpl: (async () => new Response(null, { status: 200 })) as typeof fetch,
        readSize: async () => ({ width: 640, height: 480 }),
        sensitive: true,
      },
    );
    expect(info).toMatchObject({ mimetype: "image/png", width: 640, height: 480, sensitive: true });
  });

  it("does not presign when the file type is unsupported, and fails when the upload fails", async () => {
    const { post, client } = presignClient();
    await expect(
      uploadEncryptedAttachment(client, "channel-1", file("run.exe", ""), wasmCrypto),
    ).rejects.toBeInstanceOf(AttachmentUploadError);
    expect(post).not.toHaveBeenCalled();

    await expect(
      uploadEncryptedAttachment(client, "channel-1", file(), wasmCrypto, {
        fetchImpl: (async () => new Response(null, { status: 403 })) as typeof fetch,
      }),
    ).rejects.toMatchObject({ reason: "request" });
  });
});

describe("fetchEncryptedAttachment", () => {
  it("downloads, checks the hash, decrypts, and keeps the original MIME", async () => {
    const encrypted = encryptAttachment(PLAINTEXT);
    const requestWithTx = vi.fn(async () => ({
      data: { download_url: "https://storage.example/download" },
      txid: null,
    }));
    const client = { requestWithTx } as unknown as ApiClient;
    const info = {
      attachment_id: "0b0d5c5e-6f2a-4c1e-9d7a-2f3b4c5d6e7f",
      name: "secret.txt",
      mimetype: "text/plain",
      size: PLAINTEXT.byteLength,
      file: encrypted.file,
    };
    const fetchImpl = vi.fn(
      async () => new Response(new Blob([encrypted.ciphertext as BlobPart]), { status: 200 }),
    );

    const blob = await fetchEncryptedAttachment(
      client,
      info,
      wasmCrypto,
      fetchImpl as typeof fetch,
    );
    expect(requestWithTx).toHaveBeenCalledWith(
      "/api/attachments/0b0d5c5e-6f2a-4c1e-9d7a-2f3b4c5d6e7f/url",
      { method: "GET" },
    );
    expect(fetchImpl).toHaveBeenCalledWith("https://storage.example/download", {
      credentials: "omit",
    });
    expect(blob.type).toBe("text/plain");
    expect(new Uint8Array(await blob.arrayBuffer())).toEqual(PLAINTEXT);

    // 中身を差し替えられたら、平文を返さない。
    const tampered = encrypted.ciphertext.slice();
    tampered[0]! ^= 1;
    await expect(
      fetchEncryptedAttachment(
        client,
        info,
        wasmCrypto,
        (async () => new Response(new Blob([tampered as BlobPart]))) as typeof fetch,
      ),
    ).rejects.toEqual(expect.objectContaining({ reason: "decrypt" }));
    await expect(
      fetchEncryptedAttachment(
        client,
        info,
        wasmCrypto,
        (async () => new Response(null, { status: 404 })) as typeof fetch,
      ),
    ).rejects.toBeInstanceOf(EncryptedAttachmentError);
  });
});
