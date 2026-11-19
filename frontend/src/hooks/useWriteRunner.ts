import type { Transaction } from "@tanstack/react-db";
import { useCallback, useState } from "react";

function describeError(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export interface WriteRunner {
  /** 表示用のメッセージ。`setError` で差し替えた文字列も含む。 */
  error: string | null;
  /** `run` の失敗原因の生オブジェクト。表示前に `localizeApiError` へ渡す。 */
  errorCause: unknown;
  /** 並行する書き込みがすべて確定するまで true。 */
  isPending: boolean;
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
 *
 * `error` は従来どおり文字列だが、文言をロケールに合わせる必要があるため
 * `errorCause` に失敗原因をそのまま残す。`setError` で任意の文言を入れた
 * 場合は `errorCause` を消し、呼び出し側が生の原因を誤って使わないようにする。
 */
export function useWriteRunner(): WriteRunner {
  const [pendingCount, setPendingCount] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [errorCause, setErrorCause] = useState<unknown>(null);

  const setMessage = useCallback((message: string | null) => {
    setError(message);
    setErrorCause(null);
  }, []);

  const run = useCallback(
    async (action: () => Transaction | Promise<Transaction>) => {
      setPendingCount((count) => count + 1);
      setMessage(null);
      try {
        const transaction = await action();
        await transaction.when("settled");
      } catch (cause) {
        setError(describeError(cause));
        setErrorCause(cause);
      } finally {
        setPendingCount((count) => count - 1);
      }
    },
    [setMessage],
  );

  return { error, errorCause, setError: setMessage, run, isPending: pendingCount > 0 };
}
