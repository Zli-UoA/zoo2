import { afterEach, beforeEach, describe, expect, it, vi } from "vite-plus/test";

import { BackupImportQueue } from "./backupImportQueue";

interface Leader {
  name: string;
}

function deferred() {
  let resolve!: () => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<void>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

describe("BackupImportQueue", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("実行中に担当が替わって頼まれた分は、終わったあとに今の担当で行う", async () => {
    const a: Leader = { name: "a" };
    const b: Leader = { name: "b" };
    let current: Leader | null = a;
    const first = deferred();
    const calls: string[] = [];
    const queue = new BackupImportQueue<Leader>({
      current: () => current,
      run: (target) => {
        calls.push(target.name);
        return target === a ? first.promise : Promise.resolve();
      },
      retryMs: 1000,
      retryMaxMs: 8000,
    });

    queue.request();
    current = b;
    queue.request();
    expect(calls).toEqual(["a"]);
    first.resolve();
    await vi.waitFor(() => expect(calls).toEqual(["a", "b"]));
  });

  it("担当が替わったあとで前の担当の取り込みが失敗しても、今の担当で行う", async () => {
    const a: Leader = { name: "a" };
    const b: Leader = { name: "b" };
    let current: Leader | null = a;
    const first = deferred();
    const calls: string[] = [];
    const queue = new BackupImportQueue<Leader>({
      current: () => current,
      run: (target) => {
        calls.push(target.name);
        return target === a ? first.promise : Promise.resolve();
      },
      retryMs: 1000,
      retryMaxMs: 8000,
    });

    queue.request();
    current = b;
    queue.request();
    first.reject(new Error("network error"));
    await vi.waitFor(() => expect(calls).toEqual(["a", "b"]));
    // 前の担当の試し直しは予定しない。
    await vi.advanceTimersByTimeAsync(10_000);
    expect(calls).toEqual(["a", "b"]);
  });

  it("失敗したら間隔を倍にして試し直し、成功したら間隔を戻す。担当が手放したら試し直さない", async () => {
    const a: Leader = { name: "a" };
    let current: Leader | null = a;
    let failures = 2;
    const calls: number[] = [];
    const queue = new BackupImportQueue<Leader>({
      current: () => current,
      run: async () => {
        calls.push(Date.now());
        if (failures > 0) {
          failures -= 1;
          throw new Error("network error");
        }
      },
      retryMs: 1000,
      retryMaxMs: 8000,
    });

    const start = Date.now();
    queue.request();
    await vi.advanceTimersByTimeAsync(999);
    expect(calls).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(calls).toHaveLength(2);
    await vi.advanceTimersByTimeAsync(2000);
    expect(calls).toHaveLength(3);
    expect(calls.map((t) => t - start)).toEqual([0, 1000, 3000]);

    // 担当が手放したら、予定した試し直しを取り消す。
    failures = 1;
    queue.request();
    expect(calls).toHaveLength(4);
    await vi.advanceTimersByTimeAsync(0);
    current = null;
    queue.reset();
    await vi.advanceTimersByTimeAsync(10_000);
    expect(calls).toHaveLength(4);
  });

  it("実行中に同じ担当から頼まれたら、終わったあとにもう一度行う", async () => {
    const a: Leader = { name: "a" };
    const first = deferred();
    let count = 0;
    const queue = new BackupImportQueue<Leader>({
      current: () => a,
      run: () => {
        count += 1;
        return count === 1 ? first.promise : Promise.resolve();
      },
      retryMs: 1000,
      retryMaxMs: 8000,
    });
    queue.request();
    queue.request();
    queue.request();
    first.resolve();
    await vi.waitFor(() => expect(count).toBe(2));
  });
});
