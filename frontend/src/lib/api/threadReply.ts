import { ApiError, type ApiClient, type ApiEnvelope, type RequestOptions } from "./client";

/** 最初の試行に加えて、同じ本文と id で再送する回数の上限。 */
export const THREAD_BUSY_RETRY_LIMIT = 2;

/** `Retry-After` が読めないときの待ち時間（秒。docs/threads.md の 503 に合わせる）。 */
export const DEFAULT_RETRY_AFTER_SECONDS = 5;

/**
 * スレッドのルート行ロック競合で投稿も通知も残らなかった 503。
 * ほかの 503 や 409 は再送対象にしない。
 */
export function isThreadBusyError(error: unknown): error is ApiError {
  if (!(error instanceof ApiError) || error.status !== 503) return false;
  const body = error.body;
  return (
    typeof body === "object" &&
    body !== null &&
    "code" in body &&
    (body as { code: unknown }).code === "THREAD_BUSY"
  );
}

/** 待ち時間（秒）。ヘッダーが CORS で露出されない場合は本文の `retry_after` を使う。 */
function retryAfterSeconds(error: ApiError): number {
  const body = error.body;
  const fromBody =
    typeof body === "object" && body !== null && "retry_after" in body
      ? (body as { retry_after: unknown }).retry_after
      : null;
  const seconds =
    error.retryAfter ??
    (typeof fromBody === "number" && Number.isFinite(fromBody) ? fromBody : null);
  return seconds !== null && Number.isFinite(seconds) && seconds > 0
    ? seconds
    : DEFAULT_RETRY_AFTER_SECONDS;
}

function defaultDelay(ms: number): Promise<void> {
  return new Promise((resolve) => {
    setTimeout(resolve, ms);
  });
}

export interface ThreadReplyRetryOptions extends RequestOptions {
  /** 待ち時間の注入（テスト用）。既定は `setTimeout`。 */
  delay?: (ms: number) => Promise<void>;
  /** 再送の上限（テスト用）。既定は {@link THREAD_BUSY_RETRY_LIMIT}。 */
  maxRetries?: number;
}

/**
 * スレッド返信の `POST /api/messages`。
 *
 * 503 `THREAD_BUSY` はルート行のロック取得に失敗しただけで、投稿も通知も残らない。
 * 指定秒数待ってから**同じ body（同じ id・同じ本文・同じ暗号文）**で再送する。
 * 上限を超えたら最後の `ApiError` をそのまま投げ、呼び出し側の既存のエラー表示に委ねる。
 */
export async function postThreadReply<T>(
  client: ApiClient,
  body: unknown,
  options: ThreadReplyRetryOptions = {},
): Promise<ApiEnvelope<T>> {
  const { delay = defaultDelay, maxRetries = THREAD_BUSY_RETRY_LIMIT, ...requestOptions } = options;
  // オプションが無いときは既存と同じ 2 引数で呼ぶ（他の UI のリクエスト形状を変えない）。
  const hasRequestOptions = Object.keys(requestOptions).length > 0;
  let retries = 0;
  for (;;) {
    try {
      return hasRequestOptions
        ? await client.post<T>("/api/messages", body, requestOptions)
        : await client.post<T>("/api/messages", body);
    } catch (error) {
      if (!isThreadBusyError(error) || retries >= maxRetries) throw error;
      retries += 1;
      await delay(retryAfterSeconds(error) * 1000);
    }
  }
}
