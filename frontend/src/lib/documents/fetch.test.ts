import { afterEach, expect, it, vi } from "vite-plus/test";
import { fetchPreviewBytes, PreviewLimitError } from "./fetch";
import { MAX_PREVIEW_BYTES } from "./types";
afterEach(() => vi.unstubAllGlobals());
it("uses anonymous requests and checks declared size before reading the body", async () => {
  const cancel = vi.fn();
  const body = new ReadableStream({ cancel });
  const fetch = vi
    .fn()
    .mockResolvedValue(
      new Response(body, { headers: { "content-length": String(MAX_PREVIEW_BYTES + 1) } }),
    );
  vi.stubGlobal("fetch", fetch);
  await expect(fetchPreviewBytes("https://storage.test/file")).rejects.toBeInstanceOf(
    PreviewLimitError,
  );
  expect(fetch).toHaveBeenCalledWith(
    "https://storage.test/file",
    expect.objectContaining({ credentials: "omit", referrerPolicy: "no-referrer" }),
  );
  expect(cancel).toHaveBeenCalledOnce();
});
it("enforces the streamed size limit even without a content-length header", async () => {
  const cancel = vi.fn();
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue(
      new Response(
        new ReadableStream({
          start(controller) {
            controller.enqueue(new Uint8Array(MAX_PREVIEW_BYTES + 1));
          },
          cancel,
        }),
      ),
    ),
  );
  await expect(fetchPreviewBytes("blob:source")).rejects.toBeInstanceOf(PreviewLimitError);
  expect(cancel).toHaveBeenCalledOnce();
});
