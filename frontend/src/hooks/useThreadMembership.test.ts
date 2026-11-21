// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vite-plus/test";
import { ApiError } from "../lib/api/client";
import {
  isThreadJoinRequired,
  useThreadMembership,
  type ThreadMembership,
} from "./useThreadMembership";
import { THREADS_CHANGED } from "../lib/electric/threads";
const fixture = vi.hoisted(() => ({ api: { get: vi.fn(), post: vi.fn() }, config: {} }));
vi.mock("../lib/api/client", async (original) => ({
  ...(await original<typeof import("../lib/api/client")>()),
  createConfiguredApiClient: () => fixture.api,
}));
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
  let failed: boolean | undefined;
  let retried: boolean | undefined;
  await act(async () => {
    failed = await value.change!("leave");
  });
  expect(failed).toBe(false);
  expect(value.error).toBe(true);
  expect(value.data?.membership).toBe("joined");
  await act(async () => {
    retried = await value.change!("leave");
  });
  expect(retried).toBe(true);
  expect(value.error).toBe(false);
  expect(value.data?.membership).toBe("left");
});

it("keeps the participant fields when a write answers without them", async () => {
  const withParticipants: ThreadMembership = {
    ...data,
    tracked: true,
    participant_count: 3,
    participants: [{ id: "alice", user_name: "alice", display_name: null, avatar_url: null }],
  };
  fixture.api.get.mockReset().mockResolvedValue({ data: withParticipants });
  // The participant read after the commit failed, so the response has no such fields.
  fixture.api.post.mockResolvedValue({ data: { ...data, membership: "left" } });
  await act(async () => root.render(createElement(Test)));
  expect(value.data?.participant_count).toBe(3);
  let changed: boolean | undefined;
  await act(async () => {
    changed = await value.change!("leave");
  });
  expect(changed).toBe(true);
  expect(value.data?.membership).toBe("left");
  expect(value.data?.participant_count).toBe(3);
  expect(value.data?.participants).toHaveLength(1);
  // A write is reflected without a refetch; the next periodic read corrects the kept count.
  expect(fixture.api.get).toHaveBeenCalledTimes(1);
});

it("recognises the 403 that asks for a join: the code first, the old message only without a code", () => {
  const forbidden = (body: unknown) => new ApiError(403, "forbidden", body);
  // The code wins, whatever the message says.
  expect(isThreadJoinRequired(forbidden({ error: "any text", code: "THREAD_JOIN_REQUIRED" }))).toBe(
    true,
  );
  expect(
    isThreadJoinRequired(forbidden({ error: "Join the thread before replying", code: "OTHER" })),
  ).toBe(false);
  // A server from before the code existed answers with the message only.
  expect(isThreadJoinRequired(forbidden({ error: "Join the thread before replying" }))).toBe(true);
  // A posting restriction (or any other 403) is not cured by a join.
  expect(isThreadJoinRequired(forbidden({ error: "Only channel managers may post here" }))).toBe(
    false,
  );
  expect(isThreadJoinRequired(forbidden({ code: "POSTING_RESTRICTED" }))).toBe(false);
  expect(isThreadJoinRequired(new ApiError(409, "x", { code: "THREAD_JOIN_REQUIRED" }))).toBe(
    false,
  );
  expect(isThreadJoinRequired(forbidden(null))).toBe(false);
  expect(isThreadJoinRequired(new Error("network"))).toBe(false);
});
