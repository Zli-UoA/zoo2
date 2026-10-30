// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vite-plus/test";
import { useNotificationContext, type NotificationContext } from "./useNotificationContext";

const api = vi.hoisted(() => ({ get: vi.fn() }));
vi.mock("../../../lib/api/client", () => ({ createConfiguredApiClient: () => api }));
const config = {};
vi.mock("../../../lib/electric/collections", () => ({ getCollections: () => ({ config }) }));
afterEach(() => {
  api.get.mockReset();
  vi.unstubAllGlobals();
});
const preview: NotificationContext = {
  message: { id: "message", user_id: "alice", content: "A small preview" },
  reactions: [],
};

it("fetches only a visible notification's context and refreshes on focus", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const root = createRoot(document.createElement("div"));
  api.get.mockResolvedValue(preview);
  let result!: ReturnType<typeof useNotificationContext>;
  function Probe({ id }: { id: string | null }) {
    result = useNotificationContext(id);
    return null;
  }
  try {
    await act(async () => root.render(createElement(Probe, { id: null })));
    expect(api.get).not.toHaveBeenCalled();
    await act(async () => root.render(createElement(Probe, { id: "n1" })));
    expect(result.data).toEqual(preview);
    expect(api.get.mock.calls[0][0]).toBe("/api/notifications/n1/context");
    await act(async () => window.dispatchEvent(new Event("focus")));
    expect(api.get).toHaveBeenCalledTimes(2);
    api.get.mockRejectedValueOnce(new Error("revoked"));
    await act(async () => window.dispatchEvent(new Event("focus")));
    expect(result.data).toBeNull();
    expect(result.isLoading).toBe(false);
  } finally {
    await act(async () => root.unmount());
  }
});

it("aborts stale previews and ignores their late responses", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  let resolveOld!: (value: NotificationContext) => void;
  api.get
    .mockReturnValueOnce(
      new Promise<NotificationContext>((resolve) => {
        resolveOld = resolve;
      }),
    )
    .mockResolvedValueOnce(preview);
  const root = createRoot(document.createElement("div"));
  let result!: ReturnType<typeof useNotificationContext>;
  function Probe({ id }: { id: string }) {
    result = useNotificationContext(id);
    return null;
  }
  try {
    await act(async () => root.render(createElement(Probe, { id: "old" })));
    expect(result.isLoading).toBe(true);
    const signal = api.get.mock.calls[0][1].signal as AbortSignal;
    await act(async () => root.render(createElement(Probe, { id: "new" })));
    expect(signal.aborted).toBe(true);
    expect(result.data).toEqual(preview);
    await act(async () =>
      resolveOld({ message: { id: "old", user_id: "old", content: "stale" }, reactions: [] }),
    );
    expect(result.data).toEqual(preview);
  } finally {
    await act(async () => root.unmount());
  }
});
