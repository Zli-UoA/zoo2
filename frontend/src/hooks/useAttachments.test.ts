// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vite-plus/test";
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
