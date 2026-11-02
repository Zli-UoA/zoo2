// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vite-plus/test";
import { useFederatedReactions } from "./useFederatedReactions";

const api = vi.hoisted(() => ({ get: vi.fn(), config: {}, invalidate: () => {} }));
vi.mock("../lib/api/client", () => ({ createConfiguredApiClient: () => api }));
vi.mock("../lib/electric/collections-context", () => ({
  useCollections: () => ({ config: api.config }),
}));
vi.mock("./useHomeLive", () => ({
  useHomeLive: (callback: () => void) => {
    api.invalidate = callback;
  },
}));

it("retains the overlay and reports malformed responses without breaking native reactions", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.useFakeTimers();
  const root = createRoot(document.createElement("div"));
  let result!: ReturnType<typeof useFederatedReactions>;
  function Test() {
    result = useFederatedReactions("channel-one");
    return null;
  }
  api.get.mockResolvedValue([{ id: "received" }]);
  try {
    await act(async () => root.render(createElement(Test)));
    expect(result.rows.map((row) => row.id)).toEqual(["received"]);
    expect(result.failed).toBe(false);
    api.get.mockResolvedValue({ channel_name: "wrong endpoint contract" });
    await act(async () => {
      api.invalidate();
      await vi.advanceTimersByTimeAsync(5000);
    });
    expect(result.rows.map((row) => row.id)).toEqual(["received"]);
    expect(result.failed).toBe(true);
    expect(api.get).toHaveBeenCalledTimes(2);
  } finally {
    await act(async () => root.unmount());
    vi.useRealTimers();
    vi.unstubAllGlobals();
  }
});
