import { authGenerationsChanged, currentAuthGenerations, type AuthGenerations } from "../auth";
import { ApiError, type ApiClient, type ApiEnvelope, type RequestOptions } from "./client";

/** 最初の試行に加えて、同じ本文と id で再送する回数の上限。 */
export const THREAD_BUSY_RETRY_LIMIT = 2;

/** `Retry-After` が読めないときの待ち時間（秒。docs/threads.md の 503 に合わせる）。 */
export const DEFAULT_RETRY_AFTER_SECONDS = 5;

/** `Retry-After` の待ちに上加算するジッタの上限（ミリ秒）。同時再送を分散する。 */
export const RETRY_JITTER_MAX_MS = 500;

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

/** 0〜{@link RETRY_JITTER_MAX_MS} ミリ秒の乱数。 */
function defaultJitter(): number {
  return Math.random() * RETRY_JITTER_MAX_MS;
}

export interface ThreadReplyRetryOptions extends RequestOptions {
  /** 待ち時間の注入（テスト用）。既定は `setTimeout`。 */
  delay?: (ms: number) => Promise<void>;
  /** 再送待ちに上加算するジッタ（ミリ秒を返す。テスト用）。既定は {@link defaultJitter}。 */
  jitter?: () => number;
  /** 再送の上限（テスト用）。既定は {@link THREAD_BUSY_RETRY_LIMIT}。 */
  maxRetries?: number;
}

/**
 * スレッド返信の `POST /api/messages`。
 *
 * 503 `THREAD_BUSY` はルート行のロック取得に失敗しただけで、投稿も通知も残らない。
 * `Retry-After` の秒数にランダムなジッタを足して待ち、**同じ body（同じ id・同じ本文・同じ暗号文）**で再送する。
 * ただし待機中に認証世代が変わった（ログアウト／セッションリセットなど）ときは再送せず
 * `AbortError` を投げ、古いセッションの本文が送られないようにする。
 * 上限を超えたら最後の `ApiError` をそのまま投げ、呼び出し側の既存のエラー表示に委ねる。
 */
export async function postThreadReply<T>(
  client: ApiClient,
  body: unknown,
  options: ThreadReplyRetryOptions = {},
): Promise<ApiEnvelope<T>> {
  const {
    delay = defaultDelay,
    jitter = defaultJitter,
    maxRetries = THREAD_BUSY_RETRY_LIMIT,
    ...requestOptions
  } = options;
  // オプションが無いときは既存と同じ 2 引数で呼ぶ（他の UI のリクエスト形状を変えない）。
  const hasRequestOptions = Object.keys(requestOptions).length > 0;
  // 初回送信前の世代を固定する。client.post 内のチェックは呼び出し時点の世代しか見ないため、
  // 待機中にログアウト／セッションリセットが起きると古い本文が新しいセッションで送られてしまう。
  // テスト用のスタブなどがメソッドを持たないときは auth の共通カウンタを直接読む。
  const readGenerations = (): AuthGenerations =>
    client.getAuthGenerations ? client.getAuthGenerations() : currentAuthGenerations();
  const sentGenerations = readGenerations();
  let retries = 0;
  for (;;) {
    try {
      return hasRequestOptions
        ? await client.post<T>("/api/messages", body, requestOptions)
        : await client.post<T>("/api/messages", body);
    } catch (error) {
      if (!isThreadBusyError(error) || retries >= maxRetries) throw error;
      // 503 を受けた全員が同じ時刻に再送すると競合が続くため、待ちにジッタを足す。
      await delay(retryAfterSeconds(error) * 1000 + jitter());
      // 待機中に認証世代が変わっていれば、古いセッションの本文は再送しない。
      if (authGenerationsChanged(sentGenerations, readGenerations())) {
        throw new DOMException("The operation was aborted.", "AbortError");
      }
      retries += 1;
    }
  }
}
