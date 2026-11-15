export const THREADS_CHANGED = "zoo-threads-changed";

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
