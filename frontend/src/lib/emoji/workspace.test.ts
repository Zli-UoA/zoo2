// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vite-plus/test";
import { useWorkspaceEmojis } from "./workspace";
const mocks = vi.hoisted(() => ({ config: {}, get: vi.fn() }));
vi.mock("../electric/collections-context", () => ({
  useOptionalCollections: () => ({ config: mocks.config }),
}));
vi.mock("../api/client", () => ({ createConfiguredApiClient: () => ({ get: mocks.get }) }));
let root: ReturnType<typeof createRoot>;
function Probe() {
  useWorkspaceEmojis();
  return null;
}
beforeEach(() => {
  vi.useFakeTimers();
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  mocks.config = {};
  mocks.get.mockReset().mockResolvedValue([]);
  root = createRoot(document.createElement("div"));
});
afterEach(async () => {
  await act(async () => root.unmount());
  vi.unstubAllGlobals();
  vi.useRealTimers();
});
it("reuses fresh metadata for newly mounted rows and invalidates on edits", async () => {
  await act(async () => root.render(createElement(Probe)));
  await act(async () => root.render(null));
  await act(async () => root.render(createElement(Probe)));
  expect(mocks.get).toHaveBeenCalledTimes(1);
  await act(async () => window.dispatchEvent(new Event("workspace-emojis-changed")));
  expect(mocks.get).toHaveBeenCalledTimes(2);
  await act(async () => {
    await vi.advanceTimersByTimeAsync(60000);
  });
  expect(mocks.get).toHaveBeenCalledTimes(3);
});
