import { afterEach, describe, expect, it, vi } from "vite-plus/test";

import { requestPersistentStorage, resetPersistentStorageRequest } from "./persistentStorage";

afterEach(() => resetPersistentStorageRequest());

describe("requestPersistentStorage", () => {
  it("まだ永続でなければ、ページで 1 回だけ頼む", async () => {
    const storage = { persisted: vi.fn(async () => false), persist: vi.fn(async () => false) };
    await requestPersistentStorage(storage);
    await requestPersistentStorage(storage);
    expect(storage.persist).toHaveBeenCalledTimes(1);
  });

  it("すでに永続なら頼まない", async () => {
    const storage = { persisted: vi.fn(async () => true), persist: vi.fn(async () => true) };
    await requestPersistentStorage(storage);
    expect(storage.persist).not.toHaveBeenCalled();
  });

  it("使えない、または失敗しても投げない", async () => {
    await expect(requestPersistentStorage(undefined)).resolves.toBeUndefined();
    resetPersistentStorageRequest();
    const storage = {
      persisted: vi.fn(async () => false),
      persist: vi.fn(async () => {
        throw new Error("denied");
      }),
    };
    await expect(requestPersistentStorage(storage)).resolves.toBeUndefined();
  });
});
