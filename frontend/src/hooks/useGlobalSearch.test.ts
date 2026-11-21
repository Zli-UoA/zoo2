// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vite-plus/test";

import type { SearchKind, SearchPage } from "../components/chat/search/globalSearch";
import { useGlobalSearch } from "./useGlobalSearch";

const fixture = vi.hoisted(() => ({
  api: { get: vi.fn() },
  config: {} as object,
  userId: "alice" as string | null,
}));
vi.mock("../lib/api/client", () => ({ createConfiguredApiClient: () => fixture.api }));
vi.mock("../lib/electric/collections-context", () => ({
  useCollections: () => ({ config: fixture.config }),
}));
vi.mock("./useCurrentUser", () => ({
  useCurrentUser: () => ({ currentUserId: fixture.userId }),
}));

let search: ReturnType<typeof useGlobalSearch>;
let root: ReturnType<typeof createRoot>;
function Test({ query, kind = "messages" }: { query: string; kind?: SearchKind }) {
  search = useGlobalSearch(query, kind);
  return null;
}
const item = (id: string) => ({
  id,
  channel_id: "c1",
  channel_name: "general",
  is_dm: false,
  parent_id: null,
  user_id: "u1",
  snippet: id,
  created_at: "2026-11-01T00:00:00",
});
const page = (
  ids: string[],
  next: string | null,
  until: string | null = "2025-12",
): SearchPage => ({
  items: ids.map(item),
  next_cursor: next,
  searched_until: until,
});
function deferred() {
  let resolve!: (value: SearchPage) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<SearchPage>((done, fail) => {
    resolve = done;
    reject = fail;
  });
  return { promise, resolve, reject };
}
const requested = () =>
  fixture.api.get.mock.calls.map(([path]) => new URL(String(path), "https://zoo.test"));
const flushTimers = () => act(async () => vi.runOnlyPendingTimers());

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.useFakeTimers();
  // 前のテストの結果を使わないよう、設定（キャッシュの単位）を毎回変える。
  fixture.config = {};
  fixture.userId = "alice";
  fixture.api.get.mockReset();
  root = createRoot(document.createElement("div"));
});
afterEach(async () => {
  await act(async () => root.unmount());
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

it("does not request without a query or a signed-in user", async () => {
  await act(async () => root.render(createElement(Test, { query: "" })));
  fixture.userId = null;
  await act(async () => root.render(createElement(Test, { query: "会議" })));
  expect(fixture.api.get).not.toHaveBeenCalled();
  expect(search.loaded).toBe(false);
});

it("sends one request per query and the cursor only when loading older results", async () => {
  fixture.api.get.mockResolvedValueOnce(page(["m1"], "cursor-1", "2025-12"));
  await act(async () => root.render(createElement(Test, { query: "会議" })));
  expect(fixture.api.get).toHaveBeenCalledTimes(1);
  expect(requested()[0]!.searchParams.get("cursor")).toBeNull();
  expect(search.items.map((row) => row.id)).toEqual(["m1"]);
  expect(search.searchedUntil).toBe("2025-12");

  // 取得中は 2 回目を重ねない。
  const older = deferred();
  fixture.api.get.mockReturnValueOnce(older.promise);
  act(() => search.loadMore());
  act(() => search.loadMore());
  expect(fixture.api.get).toHaveBeenCalledTimes(2);
  expect(requested()[1]!.searchParams.get("cursor")).toBe("cursor-1");
  expect(search.isLoading).toBe(true);
  await act(async () => older.resolve(page(["m1", "m2"], null, "2024-12")));
  expect(search.items.map((row) => row.id)).toEqual(["m1", "m2"]);
  expect(search.nextCursor).toBeNull();

  // 最後まで読んだら、もう読まない。
  act(() => search.loadMore());
  expect(fixture.api.get).toHaveBeenCalledTimes(2);
});

it("aborts the previous kind and ignores its late response", async () => {
  const messages = deferred();
  fixture.api.get.mockReturnValueOnce(messages.promise);
  await act(async () => root.render(createElement(Test, { query: "資料" })));
  const firstSignal = fixture.api.get.mock.calls[0]![1].signal as AbortSignal;
  fixture.api.get.mockResolvedValueOnce(page(["a1"], null));
  await act(async () => root.render(createElement(Test, { query: "資料", kind: "files" })));
  await flushTimers();
  expect(firstSignal.aborted).toBe(true);
  expect(requested().map((url) => url.searchParams.get("type"))).toEqual(["messages", "files"]);
  await act(async () => messages.resolve(page(["late"], null)));
  expect(search.items.map((row) => row.id)).toEqual(["a1"]);
});

it("reuses loaded results when the screen is opened again and keeps them on remount", async () => {
  fixture.api.get.mockResolvedValueOnce(page(["m1"], null));
  await act(async () => root.render(createElement(Test, { query: "会議" })));
  await act(async () => root.unmount());
  await flushTimers();
  root = createRoot(document.createElement("div"));
  await act(async () => root.render(createElement(Test, { query: "会議" })));
  expect(fixture.api.get).toHaveBeenCalledTimes(1);
  expect(search.items.map((row) => row.id)).toEqual(["m1"]);
});

it("keeps loaded results after a failed older page and retries from the same cursor", async () => {
  fixture.api.get
    .mockResolvedValueOnce(page(["m1"], "cursor-1"))
    .mockRejectedValueOnce(new Error("offline"))
    .mockResolvedValueOnce(page(["m2"], null));
  await act(async () => root.render(createElement(Test, { query: "会議" })));
  await act(async () => search.loadMore());
  expect(search.isError).toBe(true);
  expect(search.items.map((row) => row.id)).toEqual(["m1"]);
  await act(async () => search.loadMore());
  expect(requested().map((url) => url.searchParams.get("cursor"))).toEqual([
    null,
    "cursor-1",
    "cursor-1",
  ]);
  expect(search.items.map((row) => row.id)).toEqual(["m1", "m2"]);
  expect(search.isError).toBe(false);
});

it("retries the first page after it failed and refresh restarts from the newest month", async () => {
  fixture.api.get
    .mockRejectedValueOnce(new Error("timeout"))
    .mockResolvedValueOnce(page(["m1"], "cursor-1"))
    .mockResolvedValueOnce(page(["m0"], null));
  await act(async () => root.render(createElement(Test, { query: "会議" })));
  expect(search.isError).toBe(true);
  expect(search.loaded).toBe(false);
  await act(async () => search.loadMore());
  expect(search.items.map((row) => row.id)).toEqual(["m1"]);
  await act(async () => search.refresh());
  expect(requested().map((url) => url.searchParams.get("cursor"))).toEqual([null, null, null]);
  expect(search.items.map((row) => row.id)).toEqual(["m0"]);
});

it("retries the first page when a refresh of loaded results failed", async () => {
  fixture.api.get
    .mockResolvedValueOnce(page(["m1"], "cursor-1"))
    .mockRejectedValueOnce(new Error("timeout"))
    .mockResolvedValueOnce(page(["m3"], "cursor-3"));
  await act(async () => root.render(createElement(Test, { query: "会議" })));
  await act(async () => search.refresh());
  expect(search.isError).toBe(true);
  expect(search.items.map((row) => row.id)).toEqual(["m1"]);
  await act(async () => search.loadMore());
  // 再試行は失敗した 1 ページ目をやり直し、古い月の続き（cursor-1）は読まない。
  expect(requested().map((url) => url.searchParams.get("cursor"))).toEqual([null, null, null]);
  expect(search.items.map((row) => row.id)).toEqual(["m3"]);
  expect(search.nextCursor).toBe("cursor-3");
  expect(search.isError).toBe(false);
});

it("separates results by account", async () => {
  fixture.api.get.mockResolvedValueOnce(page(["alice"], null));
  await act(async () => root.render(createElement(Test, { query: "会議" })));
  fixture.userId = "bob";
  const bob = deferred();
  fixture.api.get.mockReturnValueOnce(bob.promise);
  await act(async () => root.render(createElement(Test, { query: "会議" })));
  expect(search.items).toEqual([]);
  expect(search.isLoading).toBe(true);
  await act(async () => bob.resolve(page(["bob"], null)));
  expect(search.items.map((row) => row.id)).toEqual(["bob"]);
});
