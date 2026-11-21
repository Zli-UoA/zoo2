import { describe, expect, it, vi } from "vite-plus/test";

import type { ApiClient } from "./client";
import {
  AttachmentDownloadError,
  AttachmentUploadError,
  fetchAttachmentBlob,
  fetchAttachmentWithRetry,
  uploadAttachment,
} from "./attachments";

function fakeFile(overrides: Partial<File> = {}): File {
  return {
    name: "screenshot.png",
    type: "image/png",
    size: 4,
    ...overrides,
  } as File;
}

describe("uploadAttachment", () => {
  it("presigns, uploads the bytes with the declared content type, and returns the attachment", async () => {
    const post = vi.fn(async () => ({
      data: {
        attachment_id: "attachment-1",
        upload_url: "https://storage.example/upload",
        storage_key: "attachments/channel-1/attachment-1",
      },
      txid: null,
    }));
    const fetchImpl = vi.fn(async () => new Response(null, { status: 200 }));
    const client = { post } as unknown as ApiClient;
    const file = fakeFile();

    await expect(
      uploadAttachment(client, "channel-1", file, fetchImpl as typeof fetch),
    ).resolves.toEqual({
      id: "attachment-1",
      file_name: "screenshot.png",
      file_size: 4,
      content_type: "image/png",
    });
    expect(post).toHaveBeenCalledWith("/api/attachments/presign", {
      channel_id: "channel-1",
      file_name: "screenshot.png",
      file_size: 4,
      content_type: "image/png",
    });
    expect(fetchImpl).toHaveBeenCalledWith("https://storage.example/upload", {
      method: "PUT",
      headers: { "Content-Type": "image/png" },
      body: file,
    });
  });

  it("uploads non-image files with a declared content type", async () => {
    const post = vi.fn(async () => ({
      data: {
        attachment_id: "attachment-2",
        upload_url: "https://attachments.zliusercontent.com/upload",
        storage_key: "attachments/channel-1/attachment-2",
      },
      txid: null,
    }));
    const fetchImpl = vi.fn(async () => new Response(null, { status: 200 }));
    const client = { post } as unknown as ApiClient;
    const file = fakeFile({ name: "release-notes.pdf", type: "application/pdf", size: 8 });

    await expect(
      uploadAttachment(client, "channel-1", file, fetchImpl as typeof fetch),
    ).resolves.toEqual({
      id: "attachment-2",
      file_name: "release-notes.pdf",
      file_size: 8,
      content_type: "application/pdf",
    });
    expect(post).toHaveBeenCalledWith("/api/attachments/presign", {
      channel_id: "channel-1",
      file_name: "release-notes.pdf",
      file_size: 8,
      content_type: "application/pdf",
    });
  });

  it("rejects executable files before requesting a presigned URL", async () => {
    const post = vi.fn();
    const client = { post } as unknown as ApiClient;

    await expect(
      uploadAttachment(
        client,
        "channel-1",
        fakeFile({ name: "setup.exe", type: "application/x-msdownload" }),
      ),
    ).rejects.toMatchObject({ reason: "unsupported" });
    expect(post).not.toHaveBeenCalled();
  });

  it("wraps presign and upload failures as request errors", async () => {
    const client = {
      post: vi.fn(async () => {
        throw new Error("network");
      }),
    } as unknown as ApiClient;

    await expect(uploadAttachment(client, "channel-1", fakeFile())).rejects.toBeInstanceOf(
      AttachmentUploadError,
    );

    const okClient = {
      post: vi.fn(async () => ({
        data: {
          attachment_id: "attachment-1",
          upload_url: "https://storage.example/upload",
          storage_key: "attachments/channel-1/attachment-1",
        },
        txid: null,
      })),
    } as unknown as ApiClient;
    const fetchImpl = vi.fn(async () => new Response(null, { status: 403 }));

    await expect(
      uploadAttachment(okClient, "channel-1", fakeFile(), fetchImpl as typeof fetch),
    ).rejects.toMatchObject({ reason: "request" });
  });
});

describe("fetchAttachmentBlob", () => {
  it("downloads the attachment bytes without credentials", async () => {
    const fetchImpl = vi.fn(async () => new Response(new Blob(["attachment"]), { status: 200 }));

    const blob = await fetchAttachmentBlob(
      "https://attachments.zliusercontent.com/object?signature=test",
      fetchImpl as typeof fetch,
    );

    expect(await blob.text()).toBe("attachment");
    expect(fetchImpl).toHaveBeenCalledWith(
      "https://attachments.zliusercontent.com/object?signature=test",
      { credentials: "omit" },
    );
  });

  it("rejects non-success responses with the status", async () => {
    const fetchImpl = vi.fn(async () => new Response(null, { status: 403 }));

    await expect(
      fetchAttachmentBlob(
        "https://attachments.zliusercontent.com/object",
        fetchImpl as typeof fetch,
      ),
    ).rejects.toMatchObject({ reason: "http", status: 403 });
  });

  it("classifies a body read failure as a network error", async () => {
    const fetchImpl = vi.fn(async () => ({
      ok: true,
      status: 200,
      blob: async () => {
        throw new TypeError("network error");
      },
    }));

    await expect(
      fetchAttachmentBlob(
        "https://attachments.zliusercontent.com/object",
        fetchImpl as unknown as typeof fetch,
      ),
    ).rejects.toMatchObject({ reason: "network", status: null });
  });
});

describe("fetchAttachmentWithRetry", () => {
  it("refetches the URL and retries once after a failed download", async () => {
    const resolveUrl = vi.fn(async ({ refresh }: { refresh: boolean }) =>
      refresh ? "https://storage.example/fresh" : "https://storage.example/old",
    );
    const fetchImpl = vi
      .fn()
      .mockResolvedValueOnce(new Response(null, { status: 403 }))
      .mockResolvedValueOnce(new Response(new Blob(["attachment"]), { status: 200 }));

    const blob = await fetchAttachmentWithRetry(resolveUrl, {
      fetchImpl: fetchImpl as typeof fetch,
      retryDelayMs: 0,
    });

    expect(await blob.text()).toBe("attachment");
    expect(resolveUrl.mock.calls).toEqual([[{ refresh: false }], [{ refresh: true }]]);
    expect(fetchImpl).toHaveBeenNthCalledWith(1, "https://storage.example/old", {
      credentials: "omit",
    });
    expect(fetchImpl).toHaveBeenNthCalledWith(2, "https://storage.example/fresh", {
      credentials: "omit",
    });
  });

  it("retries when the URL is unavailable on the first attempt", async () => {
    const resolveUrl = vi
      .fn()
      .mockRejectedValueOnce(new AttachmentDownloadError("url", { status: 503 }))
      .mockResolvedValueOnce("https://storage.example/object");
    const fetchImpl = vi.fn(async () => new Response(new Blob(["attachment"]), { status: 200 }));

    const blob = await fetchAttachmentWithRetry(resolveUrl, {
      fetchImpl: fetchImpl as typeof fetch,
      retryDelayMs: 0,
    });

    expect(await blob.text()).toBe("attachment");
    expect(resolveUrl.mock.calls).toEqual([[{ refresh: false }], [{ refresh: true }]]);
    expect(fetchImpl).toHaveBeenCalledTimes(1);
  });

  it("keeps the HTTP status when the retry also fails", async () => {
    const resolveUrl = vi.fn(async () => "https://storage.example/object");
    const fetchImpl = vi.fn(async () => new Response(null, { status: 503 }));

    const error = await fetchAttachmentWithRetry(resolveUrl, {
      fetchImpl: fetchImpl as typeof fetch,
      retryDelayMs: 0,
    }).catch((cause: unknown) => cause);

    expect(error).toBeInstanceOf(AttachmentDownloadError);
    expect(error).toMatchObject({ reason: "http", status: 503 });
  });

  it("distinguishes a network failure from a missing URL", async () => {
    const network = await fetchAttachmentWithRetry(async () => "https://storage.example/object", {
      fetchImpl: (async () => {
        throw new TypeError("Failed to fetch");
      }) as typeof fetch,
      retryDelayMs: 0,
    }).catch((cause: unknown) => cause);
    expect(network).toMatchObject({ reason: "network", status: null });

    const unavailable = await fetchAttachmentWithRetry(
      async () => {
        throw new AttachmentDownloadError("url", { status: 503 });
      },
      { retryDelayMs: 0 },
    ).catch((cause: unknown) => cause);
    expect(unavailable).toMatchObject({ reason: "url", status: 503 });
  });
});
