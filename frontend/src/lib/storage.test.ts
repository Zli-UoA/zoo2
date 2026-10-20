import { afterEach, expect, it, vi } from "vite-plus/test";

import { readStorageValue, subscribeStorage, writeStorageValue } from "./storage";

afterEach(() => vi.unstubAllGlobals());

it("handles blocked access to the storage property and blocked reads/writes", () => {
  vi.stubGlobal("window", {
    get localStorage() {
      throw new Error("blocked");
    },
  });
  expect(readStorageValue("zoo-locale")).toBe(null);
  expect(() => writeStorageValue("zoo-locale", "en")).not.toThrow();
  expect(
    readStorageValue("key", {
      getItem() {
        throw new Error("blocked");
      },
    }),
  ).toBe(null);
});

it("notifies only matching keys or clear, and unsubscribes cleanly", () => {
  const target = new EventTarget();
  vi.stubGlobal("window", target);
  const notify = vi.fn();
  const unsubscribe = subscribeStorage("zoo-locale", notify);
  const dispatch = (key: string | null) =>
    target.dispatchEvent(Object.assign(new Event("storage"), { key }));
  dispatch("other");
  expect(notify).not.toHaveBeenCalled();
  dispatch("zoo-locale");
  dispatch(null);
  expect(notify).toHaveBeenCalledTimes(2);
  unsubscribe();
  dispatch(null);
  expect(notify).toHaveBeenCalledTimes(2);
});
