import type { OidcClient, ReauthRequest } from "../oidc/client";

/**
 * E2EE の直前の再認証（docs/e2ee.md の「初回設定と解錠の流れ」の 7。#360）。
 *
 * 今の画面を離れずに、別のウィンドウで `max_age=0` の認可リクエストを開き、リダイレクト先の
 * `/auth/reauth-callback.html` から `postMessage`（と同じオリジンの BroadcastChannel）で認可コードを
 * 受け取る。メモリーにある鍵とリカバリーキーは消えない。
 */

export const REAUTH_CALLBACK_PATH = "/auth/reauth-callback.html";
export const REAUTH_CHANNEL = "zoo-reauth";
/** 利用者がログインし直すのを待つ時間。過ぎたら、もう一度開くよう求める。 */
export const REAUTH_TIMEOUT_MS = 10 * 60 * 1000;

export type ReauthErrorCode =
  /** 別のウィンドウを開けなかった（ポップアップを止められた）。利用者の操作で開き直す。 */
  | "popupBlocked"
  /** AuthZ がエラーを返した、または利用者が取り消した。 */
  | "denied"
  | "timeout"
  | "aborted";

export class ReauthError extends Error {
  constructor(
    readonly code: ReauthErrorCode,
    message: string,
  ) {
    super(message);
    this.name = "ReauthError";
  }
}

interface ReauthMessage {
  type: "zoo-reauth";
  code: string | null;
  state: string | null;
  error: string | null;
}

function isReauthMessage(value: unknown): value is ReauthMessage {
  return (
    typeof value === "object" &&
    value !== null &&
    (value as { type?: unknown }).type === "zoo-reauth"
  );
}

export interface ReauthEnvironment {
  origin: string;
  /** メッセージを受け取る（`window.addEventListener("message")`）。戻り値で解除する。 */
  onMessage(listener: (event: MessageEvent) => void): () => void;
  /** 同じオリジンの BroadcastChannel。ない環境では `null`。 */
  openChannel(): { onmessage: ((event: MessageEvent) => void) | null; close(): void } | null;
  setTimeout(callback: () => void, ms: number): () => void;
}

export function browserReauthEnvironment(): ReauthEnvironment {
  return {
    origin: window.location.origin,
    onMessage(listener) {
      window.addEventListener("message", listener);
      return () => window.removeEventListener("message", listener);
    },
    openChannel() {
      return typeof BroadcastChannel === "undefined" ? null : new BroadcastChannel(REAUTH_CHANNEL);
    },
    setTimeout(callback, ms) {
      const timer = window.setTimeout(callback, ms);
      return () => window.clearTimeout(timer);
    },
  };
}

/**
 * 別のウィンドウを開く。ポップアップを止められないよう、ボタンのクリックの中で同期的に呼ぶ。
 * 開けなければ `null`。
 */
export function openReauthWindow(): Window | null {
  return window.open("about:blank", "zoo-reauth", "popup,width=480,height=720");
}

/**
 * 開いておいたウィンドウで再認証し、ID トークンを返す。
 *
 * - `window.opener` は AuthZ を経由すると切れることがある（Cross-Origin-Opener-Policy）ので、
 *   ウィンドウが閉じたことでは失敗にしない。待つのは `REAUTH_TIMEOUT_MS` までと、`signal` まで。
 * - 受け取るのは、自分のオリジンからで、`state` が一致するメッセージだけ。
 */
export async function reauthenticate(options: {
  oidc: Pick<OidcClient, "createReauthRequest" | "exchangeReauthCode">;
  popup: Window | null;
  signal?: AbortSignal;
  environment?: ReauthEnvironment;
  timeoutMs?: number;
}): Promise<string> {
  const { oidc, popup } = options;
  if (!popup) throw new ReauthError("popupBlocked", "別のウィンドウを開けませんでした");
  const environment = options.environment ?? browserReauthEnvironment();
  let request: ReauthRequest;
  try {
    request = await oidc.createReauthRequest(
      new URL(REAUTH_CALLBACK_PATH, environment.origin).toString(),
    );
  } catch (error) {
    popup.close();
    throw error;
  }
  popup.location.href = request.url;

  const code = await new Promise<string>((resolve, reject) => {
    const cleanups: (() => void)[] = [];
    const finish = (result: { code: string } | { error: ReauthError }) => {
      for (const cleanup of cleanups) cleanup();
      try {
        popup.close();
      } catch {
        // すでに閉じている。
      }
      if ("code" in result) resolve(result.code);
      else reject(result.error);
    };
    const receive = (data: unknown) => {
      if (!isReauthMessage(data) || data.state !== request.state) return;
      if (data.error || !data.code) {
        finish({ error: new ReauthError("denied", data.error ?? "認可コードがありません") });
      } else {
        finish({ code: data.code });
      }
    };
    cleanups.push(
      environment.onMessage((event) => {
        if (event.origin !== environment.origin) return;
        receive(event.data);
      }),
    );
    const channel = environment.openChannel();
    if (channel) {
      channel.onmessage = (event) => receive(event.data);
      cleanups.push(() => channel.close());
    }
    cleanups.push(
      environment.setTimeout(
        () => finish({ error: new ReauthError("timeout", "再認証が時間内に終わりませんでした") }),
        options.timeoutMs ?? REAUTH_TIMEOUT_MS,
      ),
    );
    if (options.signal) {
      if (options.signal.aborted) {
        finish({ error: new ReauthError("aborted", "再認証を取り消しました") });
        return;
      }
      const abort = () => finish({ error: new ReauthError("aborted", "再認証を取り消しました") });
      options.signal.addEventListener("abort", abort, { once: true });
      cleanups.push(() => options.signal?.removeEventListener("abort", abort));
    }
  });
  return oidc.exchangeReauthCode(request, code);
}
