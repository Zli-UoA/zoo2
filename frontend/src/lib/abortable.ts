/**
 * 共有している要求を取り消さずに、この待機だけを `AbortSignal` で終える。
 *
 * 署名付き URL の取得はプレビューなども使う共有の要求なので、元の Promise は続ける。
 * 中止した後に元の Promise が失敗しても、ここで受け取って未処理の拒否にしない。
 */
export function abortable<T>(promise: Promise<T>, signal: AbortSignal | undefined): Promise<T> {
  if (!signal) return promise;
  if (signal.aborted) return Promise.reject(signal.reason);
  return new Promise<T>((resolve, reject) => {
    const onAbort = () => reject(signal.reason);
    signal.addEventListener("abort", onAbort, { once: true });
    promise.then(
      (value) => {
        signal.removeEventListener("abort", onAbort);
        resolve(value);
      },
      (error: unknown) => {
        signal.removeEventListener("abort", onAbort);
        reject(error);
      },
    );
  });
}
