// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vite-plus/test";
import { useAuthZAccounts } from "./accounts";

const fixture = vi.hoisted(() => ({ config: {}, get: vi.fn() }));
vi.mock("../api/client", () => ({ createConfiguredApiClient: () => ({ get: fixture.get }) }));
vi.mock("../electric/collections-context", () => ({ useOptionalCollections: () => fixture }));
const roots: ReturnType<typeof createRoot>[] = [];
function Consumer() {
  useAuthZAccounts();
  return null;
}
async function mount() {
  const root = createRoot(document.createElement("div"));
  roots.push(root);
  await act(async () => root.render(createElement(Consumer)));
  return root;
}
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.useFakeTimers();
  fixture.config = {};
  fixture.get.mockReset().mockResolvedValue({ accounts: [] });
});
afterEach(async () => {
  await act(async () => roots.splice(0).forEach((root) => root.unmount()));
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});
it("shares one timer across staggered consumers and stops after the last one leaves", async () => {
  const first = await mount();
  await act(async () => vi.advanceTimersByTimeAsync(5_000));
  const second = await mount();
  expect(fixture.get).toHaveBeenCalledTimes(1);
  await act(async () => first.render(null));
  await act(async () => vi.advanceTimersByTimeAsync(55_000));
  expect(fixture.get).toHaveBeenCalledTimes(2);
  await act(async () => second.render(null));
  await act(async () => vi.advanceTimersByTimeAsync(120_000));
  expect(fixture.get).toHaveBeenCalledTimes(2);
});
it("does not start requests while hidden and cancels the final consumer's pending request", async () => {
  vi.spyOn(document, "visibilityState", "get").mockReturnValue("hidden");
  await mount();
  await act(async () => vi.advanceTimersByTimeAsync(120_000));
  expect(fixture.get).not.toHaveBeenCalled();
  vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
  fixture.get.mockImplementation(
    (_url, { signal }: { signal: AbortSignal }) =>
      new Promise((_done, reject) => {
        signal.addEventListener("abort", () => reject(signal.reason));
      }),
  );
  await act(async () => document.dispatchEvent(new Event("visibilitychange")));
  expect(fixture.get).toHaveBeenCalledTimes(1);
  const signal = fixture.get.mock.calls[0][1].signal as AbortSignal;
  await act(async () => roots[0].render(null));
  expect(signal.aborted).toBe(true);
});
