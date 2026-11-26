export const THREADS_CHANGED = "zoo-threads-changed";
/** 段階移行中、未移行の子スレッドへの返信がDBで最上位へ正規化されたときに送出する。 */
export const THREAD_REPLY_REDIRECTED = "zoo-thread-reply-redirected";
export interface ThreadReplyRedirect {
  from: string;
  to: string;
}

// 購読者がいない間の書き込みも、次の展開時にキャッシュへ反映する。
let revision = 0;
if (typeof window !== "undefined") {
  window.addEventListener(THREADS_CHANGED, () => {
    revision += 1;
  });
}
export function threadRevision() {
  return revision;
}
