/** サーバーの画面を取得するときの時間の上限。 */
export const SHELL_FETCH_TIMEOUT_MS = 15_000;

/** 本文に書かれた入口のスクリプト (`/assets/index-<hash>.js`)。`sw.js` の `SHELL_ENTRY` と同じ見方。 */
const ENTRY = /\/assets\/index-[^"'?#]+\.js/;

/** いま動いている画面の入口のスクリプト。開発用のサーバー (Vite) では null。 */
export function currentShellEntry(
  doc: Pick<Document, "querySelector" | "baseURI"> = document,
): string | null {
  const script = doc.querySelector<HTMLScriptElement>(
    'script[type="module"][src*="/assets/index-"]',
  );
  if (!script) return null;
  return new URL(script.src, doc.baseURI).pathname;
}

/** HTML から入口のスクリプトを取り出す。 */
export function parseShellEntry(html: string): string | null {
  return html.match(ENTRY)?.[0] ?? null;
}

/**
 * サーバーにある画面 (`/`) の入口のスクリプト。`/` は Service Worker が保存しない
 * (ナビゲーションではない fetch はそのままネットワークへ行く) ので、デプロイされた版が分かる。
 */
export async function fetchDeployedShellEntry(
  fetchImpl: typeof fetch = fetch,
  timeoutMs: number = SHELL_FETCH_TIMEOUT_MS,
): Promise<string | null> {
  // 本文の受信が止まっても確認が終わるように、本文を読み終えるまでを時間で区切る。
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  try {
    const response = await fetchImpl("/", {
      cache: "no-store",
      credentials: "same-origin",
      signal: controller.signal,
    });
    if (!response.ok) return null;
    return parseShellEntry(await response.text());
  } finally {
    clearTimeout(timer);
  }
}

/** 新しい版がデプロイされているか。分からないとき (開発用のサーバー、取得の失敗) は false。 */
export async function isShellOutdated(
  current: string | null = currentShellEntry(),
  fetchImpl: typeof fetch = fetch,
): Promise<boolean> {
  if (!current) return false;
  try {
    const deployed = await fetchDeployedShellEntry(fetchImpl);
    return deployed !== null && deployed !== current;
  } catch {
    return false;
  }
}
