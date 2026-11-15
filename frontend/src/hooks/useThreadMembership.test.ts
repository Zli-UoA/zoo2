// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vite-plus/test";
import { useThreadMembership, type ThreadMembership } from "./useThreadMembership";
import { THREADS_CHANGED } from "../lib/electric/threads";
const fixture = vi.hoisted(() => ({ api: { get: vi.fn(), post: vi.fn() }, config: {} }));
vi.mock("../lib/api/client", () => ({ createConfiguredApiClient: () => fixture.api }));
vi.mock("../lib/electric/collections-context", () => ({
  useCollections: () => ({ config: fixture.config }),
}));
const data: ThreadMembership = {
  id: "thread",
  channel_id: "channel",
  membership: "joined",
  thread_archive_at: null,
  can_participate: true,
};
let value: ReturnType<typeof useThreadMembership>;
let root: ReturnType<typeof createRoot>;
function Test({ user = "alice", duplicate = false }: { user?: string; duplicate?: boolean }) {
  value = useThreadMembership(user, "thread");
  useThreadMembership(user, duplicate ? "thread" : null);
  return null;
}
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.useFakeTimers();
  fixture.config = {};
  fixture.api.get.mockReset().mockResolvedValue({ data });
  fixture.api.post.mockReset();
  root = createRoot(document.createElement("div"));
});
afterEach(async () => {
  await act(async () => root.unmount());
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});
it("shares requests and timers, stops while hidden, and resumes on visibility", async () => {
  const visibility = vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
  await act(async () => root.render(createElement(Test, { duplicate: true })));
  expect(fixture.api.get).toHaveBeenCalledTimes(1);
  await act(async () => vi.advanceTimersByTimeAsync(60_000));
  expect(fixture.api.get).toHaveBeenCalledTimes(2);
  visibility.mockReturnValue("hidden");
  await act(async () => document.dispatchEvent(new Event("visibilitychange")));
  await act(async () => vi.advanceTimersByTimeAsync(120_000));
  expect(fixture.api.get).toHaveBeenCalledTimes(2);
  visibility.mockReturnValue("visible");
  await act(async () => document.dispatchEvent(new Event("visibilitychange")));
  expect(fixture.api.get).toHaveBeenCalledTimes(3);
});
it("coalesces notifications while a slow request is running", async () => {
  let resolve!: (v: unknown) => void;
  fixture.api.get.mockImplementationOnce(
    () =>
      new Promise((done) => {
        resolve = done;
      }),
  );
  await act(async () => root.render(createElement(Test)));
  await act(async () => {
    for (let i = 0; i < 5; i++) window.dispatchEvent(new Event(THREADS_CHANGED));
  });
  expect(fixture.api.get).toHaveBeenCalledTimes(1);
  await act(async () => resolve({ data }));
  expect(fixture.api.get).toHaveBeenCalledTimes(2);
  expect(value.data).toEqual(data);
});
it("deduplicates leave and ignores an older read", async () => {
  let read!: (v: unknown) => void;
  let leave!: (v: unknown) => void;
  fixture.api.get.mockImplementationOnce(
    () =>
      new Promise((done) => {
        read = done;
      }),
  );
  fixture.api.post.mockImplementationOnce(
    () =>
      new Promise((done) => {
        leave = done;
      }),
  );
  await act(async () => root.render(createElement(Test)));
  await act(async () => {
    void value.change!("leave");
    void value.change!("leave");
  });
  expect(fixture.api.post).toHaveBeenCalledTimes(1);
  await act(async () => leave({ data: { ...data, membership: "left" } }));
  await act(async () => read({ data }));
  expect(value.data?.membership).toBe("left");
  expect(value.pending).toBe(false);
  expect(fixture.api.get).toHaveBeenCalledTimes(1);
});
it("isolates a late mutation after switching accounts", async () => {
  let leave!: (v: unknown) => void;
  fixture.api.post.mockImplementationOnce(
    () =>
      new Promise((done) => {
        leave = done;
      }),
  );
  await act(async () => root.render(createElement(Test)));
  await act(async () => {
    void value.change!("leave");
  });
  await act(async () => root.render(createElement(Test, { user: "bob" })));
  await act(async () => leave({ data: { ...data, membership: "left" } }));
  expect(value.data?.membership).toBe("joined");
  expect(fixture.api.get).toHaveBeenCalledTimes(2);
});
it("keeps data on a failed operation and allows retry", async () => {
  fixture.api.post
    .mockRejectedValueOnce(new Error("offline"))
    .mockResolvedValue({ data: { ...data, membership: "left" } });
  await act(async () => root.render(createElement(Test)));
  await act(async () => value.change!("leave"));
  expect(value.error).toBe(true);
  expect(value.data?.membership).toBe("joined");
  await act(async () => value.change!("leave"));
  expect(value.error).toBe(false);
  expect(value.data?.membership).toBe("left");
});
