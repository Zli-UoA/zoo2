/**
 * デスクトップ版（Tauri, `desktop-tauri/`）の中で、zli.chat がリモートで読み込まれたときの IPC。
 *
 * - OIDC の代わりに、Rust 側が持つトークンを `zoo_access_token` で受け取る。
 * - OS の通知は Rust 側が出すので、通知のコレクションの変化を `zoo_notifications_observed` で渡す。
 * - 判定は `window.__TAURI_INTERNALS__` があり、`zoo_session` の `protocol` が一致して
 *   `ui` が `remote` のときだけ。合わなければ通常の Web として動く（Web は Tauri 版と別にデプロイされるため）。
 *
 * 型と名前は `client-types/src/ipc.rs` と同じ。互換を崩すときは両方の PROTOCOL_VERSION を上げる。
 */

export const DESKTOP_PROTOCOL_VERSION = 1;

type TauriInternals = {
  invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;
  transformCallback: (callback: (payload: unknown) => void, once?: boolean) => number;
};

type DesktopSessionInfo = {
  protocol: number;
  ui: "leptos" | "remote";
  api_base_url: string;
  state: { state: string };
};

type DesktopAccessToken = { access_token: string; expires_at: number };

/** Rust 側の `shape::Change`。`row` は Electric の行と同じ列名を持つ。 */
export type DesktopChange =
  | { op: "upsert"; key: string; row: Record<string, unknown> }
  | { op: "delete"; key: string };

export type DesktopBatch = { reset: boolean; changes: DesktopChange[]; up_to_date: boolean };

export type DesktopOpenTarget = { channel_id: string; message_id: string | null };

/** 期限のこの時間前までは、受け取ったトークンを使い回す。Rust 側の更新の余裕と同じ。 */
const TOKEN_SKEW_MS = 60_000;

function internals(): TauriInternals | null {
  if (typeof window === "undefined") return null;
  return (
    (window as unknown as { __TAURI_INTERNALS__?: TauriInternals }).__TAURI_INTERNALS__ ?? null
  );
}

/** Tauri の WebView の中にいるか（プロトコルの確認の前に、同期で分かる部分）。 */
export function hasDesktopBridge(): boolean {
  return internals() !== null;
}

let detected: Promise<boolean> | null = null;
let isDesktop = false;

/** デスクトップ版の (b) として動くかを確かめる。結果は覚えておく。 */
export function detectDesktop(): Promise<boolean> {
  detected ??= (async () => {
    const bridge = internals();
    if (!bridge) return false;
    try {
      const info = (await bridge.invoke("zoo_session")) as DesktopSessionInfo;
      isDesktop = info.protocol === DESKTOP_PROTOCOL_VERSION && info.ui === "remote";
    } catch {
      isDesktop = false;
    }
    return isDesktop;
  })();
  return detected;
}

/** `detectDesktop` が済んだ後の結果。済む前は false。 */
export function isDesktopSession(): boolean {
  return isDesktop;
}

function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const bridge = internals();
  if (!bridge || !isDesktop) return Promise.reject(new Error("デスクトップ版の中ではありません"));
  return bridge.invoke(cmd, args) as Promise<T>;
}

let cachedToken: { token: string; expiresAtMs: number } | null = null;

/**
 * `setTokenProvider` に渡す。ログインしていなければ null（呼び出し側は 401 として扱い、
 * ログアウトする）。IdP の一時的な障害などそれ以外の失敗では throw し、リクエストを
 * 通信のエラーとして失敗させる（null にすると 401 からログアウトしてトークンを消してしまう）。
 */
export async function desktopAccessToken(): Promise<string | null> {
  if (cachedToken && cachedToken.expiresAtMs - TOKEN_SKEW_MS > Date.now()) {
    return cachedToken.token;
  }
  try {
    const result = await invoke<DesktopAccessToken>("zoo_access_token");
    cachedToken = { token: result.access_token, expiresAtMs: result.expires_at * 1000 };
    return result.access_token;
  } catch (error) {
    cachedToken = null;
    if (isNotLoggedIn(error)) return null;
    throw error instanceof Error ? error : new Error(errorMessage(error));
  }
}

function isNotLoggedIn(error: unknown): boolean {
  return (error as { code?: unknown } | null)?.code === "not_logged_in";
}

function errorMessage(error: unknown): string {
  const message = (error as { message?: unknown } | null)?.message;
  return typeof message === "string" ? message : "デスクトップ版からトークンを取れませんでした";
}

/** Rust 側でトークンを消す。Rust 側がウィンドウを同梱のログイン画面へ戻す。 */
export async function desktopLogout(): Promise<void> {
  cachedToken = null;
  await invoke("zoo_logout");
}

/** 通知を抑えるため、前面かどうかと表示中のチャンネルを知らせる。 */
export function desktopSetFocus(channelId: string | null): void {
  void invoke("zoo_set_focus", {
    focus: { focused: document.hasFocus(), channel_id: channelId },
  }).catch(() => {});
}

export function desktopReportNotifications(batch: DesktopBatch): void {
  void invoke("zoo_notifications_observed", { observed: { batch } }).catch(() => {});
}

/**
 * Rust 側のイベントを購読する。`@tauri-apps/api` の `listen` と同じ呼び方で、
 * 依存を増やさないために internals を直接使う。戻り値で購読をやめる。
 */
export async function desktopListen<T>(
  event: string,
  handler: (payload: T) => void,
): Promise<() => void> {
  const bridge = internals();
  if (!bridge || !isDesktop) return () => {};
  const callback = bridge.transformCallback((message) => {
    handler((message as { payload: T }).payload);
  });
  const eventId = (await bridge.invoke("plugin:event|listen", {
    event,
    target: { kind: "Any" },
    handler: callback,
  })) as number;
  return () => {
    void bridge.invoke("plugin:event|unlisten", { event, eventId }).catch(() => {});
  };
}

/** テスト用。判定の結果とトークンのキャッシュを捨てる。 */
export function resetDesktopForTest(): void {
  detected = null;
  isDesktop = false;
  cachedToken = null;
}
