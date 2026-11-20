// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vite-plus/test";
import { AttachmentDownloadError } from "../lib/api/attachments";
import { useAttachmentUrl } from "./useAttachments";
const mocks = vi.hoisted(() => ({ config: {}, requestWithTx: vi.fn() }));
vi.mock("../lib/electric/collections-context", () => ({
  useCollections: () => ({ config: mocks.config }),
}));
vi.mock("../lib/api/client", () => ({
  createConfiguredApiClient: () => ({ requestWithTx: mocks.requestWithTx }),
}));
let root: ReturnType<typeof createRoot>;
const results: ReturnType<typeof useAttachmentUrl>[] = [];
function Probe({ index }: { index: number }) {
  results[index] = useAttachmentUrl("image");
  return null;
}
const tree = () => [
  createElement(Probe, { key: "first", index: 0 }),
  createElement(Probe, { key: "second", index: 1 }),
];
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  mocks.config = {};
  mocks.requestWithTx.mockReset();
  root = createRoot(document.createElement("div"));
});
afterEach(async () => {
  await act(async () => root.unmount());
  vi.unstubAllGlobals();
});
it("shares one URL request across image copies and reuses the signed URL", async () => {
  let finish!: (value: unknown) => void;
  mocks.requestWithTx.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  await act(async () => root.render(tree()));
  expect(mocks.requestWithTx).toHaveBeenCalledTimes(1);
  await act(async () => {
    finish({ data: { download_url: "https://files.test/image.png" }, txid: null });
  });
  expect(results.map((value) => value.url)).toEqual([
    "https://files.test/image.png",
    "https://files.test/image.png",
  ]);
  await act(async () => root.render(null));
  await act(async () => root.render(tree()));
  expect(mocks.requestWithTx).toHaveBeenCalledTimes(1);
  mocks.config = {};
  mocks.requestWithTx.mockResolvedValue({
    data: { download_url: "https://other.test/image.png" },
    txid: null,
  });
  await act(async () => root.render(tree()));
  expect(mocks.requestWithTx).toHaveBeenCalledTimes(2);
  expect(results[0].url).toBe("https://other.test/image.png");
});

it("returns the URL to a download even when the row unmounts mid-request", async () => {
  let finish!: (value: unknown) => void;
  mocks.requestWithTx.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  await act(async () => root.render(tree()));
  const getFreshUrl = results[0].getFreshUrl;
  const pending = getFreshUrl();
  // 仮想スクロールなどで、URL の取得中に行が外れる。
  await act(async () => root.render(null));
  await act(async () => {
    finish({ data: { download_url: "https://files.test/image.png" }, txid: null });
  });
  await expect(pending).resolves.toBe("https://files.test/image.png");
  expect(mocks.requestWithTx).toHaveBeenCalledTimes(1);
});

it("refetches only when getFreshUrl is asked to refresh", async () => {
  mocks.requestWithTx.mockResolvedValue({
    data: { download_url: "https://files.test/image.png" },
    txid: null,
  });
  await act(async () => root.render(tree()));
  await expect(results[0].getFreshUrl()).resolves.toBe("https://files.test/image.png");
  expect(mocks.requestWithTx).toHaveBeenCalledTimes(1);
  await expect(results[0].getFreshUrl({ refresh: true })).resolves.toBe(
    "https://files.test/image.png",
  );
  expect(mocks.requestWithTx).toHaveBeenCalledTimes(2);
});

it("keeps the /url status and marks the preview as failed", async () => {
  mocks.requestWithTx.mockRejectedValue(
    Object.assign(new Error("Request failed"), { status: 503 }),
  );
  await act(async () => root.render(tree()));
  const getFreshUrl = results[0].getFreshUrl;
  let error: unknown;
  await act(async () => {
    error = await getFreshUrl().catch((cause: unknown) => cause);
  });
  expect(error).toBeInstanceOf(AttachmentDownloadError);
  expect(error).toMatchObject({ reason: "url", status: 503 });
  expect(results[0].status).toBe("error");
});

it("does not overwrite the display state with a URL fetched for an old config", async () => {
  let finishFirst!: (value: unknown) => void;
  mocks.requestWithTx.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finishFirst = resolve;
      }),
  );
  await act(async () => root.render(tree()));
  const getFreshUrl = results[0].getFreshUrl;
  const pending = getFreshUrl();
  // 接続の設定を切り替え、新しい設定で URL を取得する。
  mocks.config = {};
  mocks.requestWithTx.mockResolvedValue({
    data: { download_url: "https://files.test/new.png" },
    txid: null,
  });
  await act(async () => root.render(tree()));
  expect(results[0].url).toBe("https://files.test/new.png");
  // 切り替え前の取得が完了しても、表示は新しい設定の URL のまま。
  await act(async () => {
    finishFirst({ data: { download_url: "https://files.test/old.png" }, txid: null });
  });
  await expect(pending).resolves.toBe("https://files.test/old.png");
  expect(results[0].url).toBe("https://files.test/new.png");
});
