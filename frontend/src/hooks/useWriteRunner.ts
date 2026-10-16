import type { Transaction } from "@tanstack/react-db";
import { useCallback, useState } from "react";

function describeError(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export interface WriteRunner {
  error: string | null;
  setError: (message: string | null) => void;
  /** 楽観更新トランザクションを確定まで待ち、失敗はエラーメッセージにまとめる。 */
  run: (action: () => Transaction | Promise<Transaction>) => Promise<void>;
}

/**
 * 画面からの書き込みをまとめて実行する。
 *
 * `collection.insert()` / `update()` / `delete()` は楽観更新の
 * `Transaction` を返す。`when("settled")` で確定まで待ち、REST 失敗や
 * `awaitTxId` のタイムアウトはロールバックとしてここで受け止める。
 */
export function useWriteRunner(): WriteRunner {
  const [error, setError] = useState<string | null>(null);

  const run = useCallback(async (action: () => Transaction | Promise<Transaction>) => {
    setError(null);
    try {
      const transaction = await action();
      await transaction.when("settled");
    } catch (cause) {
      setError(describeError(cause));
    }
  }, []);

  return { error, setError, run };
}
