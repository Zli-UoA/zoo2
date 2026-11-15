import { useTranslation } from "react-i18next";
import { useRouter } from "@tanstack/react-router";
import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";

import {
  getAuthSessionGeneration,
  notifyAuthSessionReauthorizationStarted,
  setAuthorizedHandler,
  setTokenProvider,
  setUnauthorizedHandler,
  subscribeAuthSessionEnded,
} from "../../lib/auth";
import {
  desktopAccessToken,
  desktopLogout,
  detectDesktop,
  hasDesktopBridge,
} from "../../lib/desktop";
import { resolveConfig, type AppConfig } from "../../lib/config";
import { forgetPushRecipient } from "../../lib/e2ee/push/recipient";
import { OidcClient, OidcError } from "../../lib/oidc/client";
import {
  MAX_REDIRECT_ATTEMPTS,
  clearAuthorizeRedirectPending,
  clearRedirectStarted,
  clearRedirectAttempts,
  isLoggedOut,
  markAuthorizeRedirectPending,
  markRedirectStarted,
  readLastRedirectAt,
  readUnauthorizedRedirectState,
  recordRedirectAttempt,
} from "../../lib/oidc/pending";
import { getOidcClient } from "../../lib/oidc/session";
import { LanguageToggle } from "../i18n/LanguageToggle";
import { ShellSkeleton } from "../layout/ShellSkeleton";
import { Button } from "../ui/button";

import { describeAuthError, formatAuthError, type AuthError } from "./authError";

type AuthPhase = "checking" | "ready" | "disabled" | "loggedOut" | "logoutPending" | "error";

/**
 * リダイレクト直後の 401 を再ログインせずエラーにする猶予。
 * ページをまたぐため sessionStorage の時刻で判定する。
 */
const REDIRECT_COOLDOWN_MS = 10_000;

function currentReturnTo(): string {
  if (typeof window === "undefined") {
    return "/";
  }
  const { pathname, search, hash } = window.location;
  return `${pathname}${search}${hash}`;
}

/**
 * 認証の入口。デスクトップ版（Tauri）の中では Rust 側のトークンを使い、
 * それ以外では OIDC（`WebAuthGate`）を使う。Tauri の中では OIDC へは進まない。
 */
export function AuthGate({ children }: { children: ReactNode }) {
  if (hasDesktopBridge()) return <DesktopAuthGate>{children}</DesktopAuthGate>;
  return <WebAuthGate>{children}</WebAuthGate>;
}

/**
 * デスクトップ版の (b)。ログインは Rust 側（同梱のログイン画面）で済んでいるので、
 * トークンを IPC で受け取るだけにする。401 になったら Rust 側でログアウトし、
 * Rust 側がウィンドウをログイン画面へ戻す。
 *
 * Tauri の中では OIDC のログインを完了できない（AuthZ への移動は `on_navigation` が止めて
 * 既定のブラウザーで開くため）。プロトコルが合わないときや `zoo_session` が失敗したときは
 * `WebAuthGate` へ進まず、アプリの更新を案内する。
 */
function DesktopAuthGate({ children }: { children: ReactNode }) {
  const { t } = useTranslation();
  const [mode, setMode] = useState<"checking" | "desktop" | "unsupported">("checking");
  useEffect(() => {
    let cancelled = false;
    void detectDesktop().then((desktop) => {
      if (cancelled) return;
      if (desktop) {
        setTokenProvider(desktopAccessToken);
        setUnauthorizedHandler(() => {
          void desktopLogout();
        });
      }
      setMode(desktop ? "desktop" : "unsupported");
    });
    return () => {
      cancelled = true;
      setUnauthorizedHandler(null);
    };
  }, []);
  if (mode === "checking") return <ShellSkeleton />;
  if (mode === "desktop") return <>{children}</>;
  return (
    <div className="flex h-dvh w-full items-center justify-center bg-background p-6 text-foreground">
      <div className="flex max-w-md flex-col items-center gap-4 text-center">
        <LanguageToggle />
        <h1 className="text-lg font-semibold">{t("auth.desktopOutdated")}</h1>
        <p className="text-sm text-muted-foreground">{t("auth.desktopOutdatedHelp")}</p>
        <Button onClick={() => window.location.reload()}>{t("auth.reload")}</Button>
      </div>
    </div>
  );
}

/**
 * OIDC 未設定（dev の X-Dev-User）なら素通しし、設定済みなら
 * authorize リダイレクト / コールバック処理 / 401 時の再ログインを担う。
 *
 * 同じタブの再読み込みではセッションを復元し、IdP への往復を省く。
 */
function WebAuthGate({ children }: { children: ReactNode }) {
  const { t } = useTranslation();
  const router = useRouter();
  const [phase, setPhase] = useState<AuthPhase>("checking");
  const [error, setError] = useState<AuthError | null>(null);
  const [initializationAttempt, setInitializationAttempt] = useState(0);
  const [logoutRetrying, setLogoutRetrying] = useState(false);
  const clientRef = useRef<OidcClient | null>(null);
  const authRecoveryPending = useRef(false);
  const failedRedirectAt = useRef<number | null>(null);
  const loginInProgress = useRef(false);
  const logoutRetryInProgress = useRef(false);
  const logoutBlocked = useRef(false);
  const loginAttempt = useRef(0);

  const startLogin = useCallback((client: OidcClient, returnTo: string) => {
    if (loginInProgress.current) return;
    if (typeof navigator !== "undefined" && !navigator.onLine) {
      authRecoveryPending.current = true;
      setError(describeAuthError(new OidcError("session_reset", "セッションが破棄されました")));
      setPhase("error");
      return;
    }
    if (!notifyAuthSessionReauthorizationStarted()) {
      authRecoveryPending.current = true;
      setError(describeAuthError(new OidcError("session_reset", "セッションが破棄されました")));
      setPhase("error");
      return;
    }
    const authSessionGeneration = getAuthSessionGeneration();
    const attempt = ++loginAttempt.current;
    loginInProgress.current = true;
    authRecoveryPending.current = false;
    setError(null);
    setPhase("checking");
    const now = Date.now();
    const failLogin = (cause: unknown) => {
      if (
        attempt !== loginAttempt.current ||
        authSessionGeneration !== getAuthSessionGeneration()
      ) {
        return;
      }
      loginInProgress.current = false;
      authRecoveryPending.current = true;
      // Associate the failed attempt with the marker it wrote. If storage refuses
      // removal, a matching recent marker is still recognized as pre-navigation.
      failedRedirectAt.current = now;
      try {
        clearAuthorizeRedirectPending(window.sessionStorage);
      } catch {
        // Storage may still be unavailable; keep the retry screen usable.
      }
      try {
        clearRedirectStarted(window.sessionStorage);
      } catch {
        // A stale marker must not disable retry if sessionStorage is unavailable.
      }
      setError(describeAuthError(cause));
      setPhase("error");
    };
    try {
      const storage = window.sessionStorage;
      failedRedirectAt.current = null;
      markRedirectStarted(storage, now);
      markAuthorizeRedirectPending(storage, now);
      void client.login(returnTo).catch(failLogin);
    } catch (cause) {
      failLogin(cause);
    }
  }, []);

  useEffect(() => {
    let cancelled = false;
    let client: OidcClient | null = null;
    let unsubscribeAuthSession = () => {};
    let handlePageShow: ((event: PageTransitionEvent) => void) | null = null;
    let storage: Storage;
    try {
      const config: AppConfig = resolveConfig();
      client = getOidcClient(config);
      if (!client) {
        setPhase("disabled");
        return;
      }
      // OidcClient construction reads this property too. Keep both that work and
      // the first storage operations inside the retryable initialization boundary.
      storage = window.sessionStorage;
      clearAuthorizeRedirectPending(storage);
      const loggedOut = isLoggedOut(storage);

      const activeClient = client;
      unsubscribeAuthSession = subscribeAuthSessionEnded((reason) => {
        if (reason === "reauthorize") return;
        loginAttempt.current += 1;
        loginInProgress.current = false;
        if (reason === "logoutBlocked") {
          logoutBlocked.current = true;
          authRecoveryPending.current = false;
          setError(null);
          setPhase("logoutPending");
          return;
        }
        if (reason === "logout") {
          logoutBlocked.current = false;
          failedRedirectAt.current = null;
          try {
            clearAuthorizeRedirectPending(storage);
          } catch {
            // The logged-out UI must still take effect when storage is unavailable.
          }
          authRecoveryPending.current = false;
          setError(null);
          setPhase(logoutRetryInProgress.current ? "checking" : "loggedOut");
        } else {
          authRecoveryPending.current = true;
          setError(describeAuthError(new OidcError("session_reset", "セッションが破棄されました")));
          setPhase("error");
        }
      });
      clientRef.current = activeClient;
      setTokenProvider(() => activeClient.getAccessToken());
      activeClient.setReauthorizeHandler((returnTo) => {
        startLogin(activeClient, returnTo);
      });
      setAuthorizedHandler(() => {
        // バックエンドがトークンを受け付けたら試行カウンタを戻す。
        try {
          clearRedirectAttempts(storage);
        } catch {
          // Storage cleanup must never turn a successful API response into a failure.
          // A later successful response can retry clearing the counter.
        }
      });
      setUnauthorizedHandler(() => {
        if (logoutBlocked.current) return;
        // 認証が切れたら、Service Worker が Push を復号してよい宛先を消す（docs/e2ee.md の「Web Push
        // の復号」）。ログインし直すと、E2eeProvider がそのユーザーを置き直す。別のユーザーで
        // ログインし直すまでのあいだに、前のユーザーの本文を通知に出さないため。
        void forgetPushRecipient().catch(() => undefined);
        const now = Date.now();
        let redirectState: ReturnType<typeof readUnauthorizedRedirectState>;
        try {
          if (isLoggedOut(storage)) {
            authRecoveryPending.current = false;
            setPhase("loggedOut");
            return;
          }
          redirectState = readUnauthorizedRedirectState(storage, now, REDIRECT_COOLDOWN_MS);
          if (redirectState === "recent" && failedRedirectAt.current !== null) {
            if (readLastRedirectAt(storage) === failedRedirectAt.current) {
              // This "recent" timestamp belongs to a login that failed before
              // navigation, so it cannot be evidence of a completed redirect loop.
              redirectState = "none";
            } else {
              failedRedirectAt.current = null;
            }
          } else if (redirectState !== "recent") {
            failedRedirectAt.current = null;
          }
        } catch (cause) {
          activeClient.resetSession();
          authRecoveryPending.current = true;
          setError(describeAuthError(cause));
          setPhase("error");
          return;
        }
        // 事前再認可や 401 起点のリダイレクト中に届いた 401 は、遷移前の
        // リクエストの結果。設定不一致のループ判定には使わない。
        if (redirectState === "redirecting") {
          return;
        }
        // リダイレクト直後の 401 は「IdP では通るのにバックエンドが拒否する」
        // 設定不一致の典型。再ログインせずエラー画面へ倒してループを止める。
        if (redirectState === "recent") {
          authRecoveryPending.current = false;
          setError({ kind: "unauthorizedLoop" });
          setPhase("error");
          return;
        }
        activeClient.resetSession();
        let attempts: number;
        try {
          attempts = recordRedirectAttempt(storage, now);
        } catch (cause) {
          authRecoveryPending.current = true;
          setError(describeAuthError(cause));
          setPhase("error");
          return;
        }
        if (attempts > MAX_REDIRECT_ATTEMPTS) {
          authRecoveryPending.current = false;
          setError({ kind: "unauthorizedLoop" });
          setPhase("error");
          return;
        }
        startLogin(activeClient, currentReturnTo());
      });

      // bfcache から戻った場合は mount されないため、進行中フラグを明示的に
      // 落とす。戻った先で 401 が続いても再ログインを止めないようにする。
      handlePageShow = (event: PageTransitionEvent) => {
        if (event.persisted) {
          const wasLoggingIn = loginInProgress.current;
          if (wasLoggingIn) {
            loginAttempt.current += 1;
            loginInProgress.current = false;
            activeClient.cancelPendingLogin();
          }
          try {
            if (logoutBlocked.current) return;
            if (isLoggedOut(storage)) {
              activeClient.resetSession();
              authRecoveryPending.current = false;
              setError(null);
              setPhase("loggedOut");
              return;
            }
            clearAuthorizeRedirectPending(storage);
            if (wasLoggingIn) {
              clearRedirectStarted(storage);
              if (activeClient.hasSession) {
                authRecoveryPending.current = false;
                setError(null);
                setPhase("ready");
              } else {
                authRecoveryPending.current = true;
                setError(
                  describeAuthError(new OidcError("session_reset", "セッションが破棄されました")),
                );
                setPhase("error");
              }
            }
          } catch (cause) {
            if (logoutBlocked.current) return;
            loginInProgress.current = false;
            authRecoveryPending.current = true;
            setError(describeAuthError(cause));
            setPhase("error");
          }
        }
      };
      window.addEventListener("pageshow", handlePageShow);

      if (activeClient.isCallbackUrl()) {
        const callbackAuthSessionGeneration = getAuthSessionGeneration();
        activeClient
          .handleCallback()
          .then(async ({ returnTo }) => {
            if (cancelled || callbackAuthSessionGeneration !== getAuthSessionGeneration()) {
              return;
            }
            authRecoveryPending.current = false;
            loginInProgress.current = false;
            router.history.replace(returnTo);
            // 復帰先のルート解決を待ってから描画する（コールバック URL の
            // notFound が一瞬描画されるのを防ぐ）。
            await router.load().catch(() => {});
            if (cancelled || callbackAuthSessionGeneration !== getAuthSessionGeneration()) {
              return;
            }
            setPhase("ready");
          })
          .catch((cause: unknown) => {
            if (cancelled || callbackAuthSessionGeneration !== getAuthSessionGeneration()) {
              return;
            }
            loginInProgress.current = false;
            // code / error を含むコールバック URL を残すと、リロードのたびに
            // 同じ失敗を繰り返すため、エラー画面へ移る前に置き換える。
            router.history.replace("/");
            void router.load().catch(() => {});
            let loggedOutNow = false;
            try {
              loggedOutNow = isLoggedOut(storage);
            } catch {
              loggedOutNow = true;
            }
            if (loggedOutNow) {
              authRecoveryPending.current = false;
              setError(null);
              setPhase("loggedOut");
              return;
            }
            setError(describeAuthError(cause));
            setPhase("error");
          });
      } else if (activeClient.hasSession) {
        setPhase("ready");
      } else if (loggedOut) {
        setPhase("loggedOut");
      } else if (typeof navigator !== "undefined" && !navigator.onLine) {
        // オフラインでは IdP に到達できない。トークンが無くても、保存済みの
        // チャンネル・メッセージを読めるようアプリを起動する。
        setPhase("ready");
      } else {
        startLogin(activeClient, currentReturnTo());
      }
    } catch (cause) {
      if (client) {
        clientRef.current = null;
        client.setReauthorizeHandler(null);
      }
      unsubscribeAuthSession();
      if (handlePageShow) window.removeEventListener("pageshow", handlePageShow);
      setUnauthorizedHandler(null);
      setAuthorizedHandler(null);
      authRecoveryPending.current = true;
      setError(describeAuthError(cause));
      setPhase("error");
    }

    return () => {
      cancelled = true;
      if (handlePageShow) window.removeEventListener("pageshow", handlePageShow);
      loginAttempt.current += 1;
      loginInProgress.current = false;
      unsubscribeAuthSession();
      setUnauthorizedHandler(null);
      setAuthorizedHandler(null);
      client?.setReauthorizeHandler(null);
      if (clientRef.current === client) clientRef.current = null;
    };
  }, [initializationAttempt, router, startLogin]);

  // オフライン起動から接続が戻ったら、通常の OIDC ログインへ戻す。
  // 認証付き API が 401 を返すまでの間、キャッシュ表示を止めない。
  useEffect(() => {
    const client = clientRef.current;
    if (!client) {
      return;
    }
    let active = true;
    const handleOnline = () => {
      if (phase === "loggedOut" || phase === "logoutPending") return;
      if (!authRecoveryPending.current && phase !== "ready") return;
      let loggedOut: boolean;
      try {
        loggedOut = isLoggedOut(window.sessionStorage);
      } catch (cause) {
        authRecoveryPending.current = true;
        setError(describeAuthError(cause));
        setPhase("error");
        return;
      }
      if (loggedOut) return;
      if (client.hasSession && !authRecoveryPending.current) {
        void client
          .getAccessToken()
          .then((token) => {
            if (!active || token || !navigator.onLine || loginInProgress.current) return;
            try {
              if (isLoggedOut(window.sessionStorage)) return;
              if (
                readUnauthorizedRedirectState(
                  window.sessionStorage,
                  Date.now(),
                  REDIRECT_COOLDOWN_MS,
                ) === "redirecting"
              )
                return;
            } catch (cause) {
              authRecoveryPending.current = true;
              setError(describeAuthError(cause));
              setPhase("error");
              return;
            }
            startLogin(client, currentReturnTo());
          })
          .catch((cause: unknown) => {
            if (!active) return;
            authRecoveryPending.current = true;
            setError(describeAuthError(cause));
            setPhase("error");
          });
        return;
      }
      if (loggedOut) return;
      setPhase("checking");
      startLogin(client, currentReturnTo());
    };
    window.addEventListener("online", handleOnline);
    return () => {
      active = false;
      window.removeEventListener("online", handleOnline);
    };
  }, [phase, startLogin]);

  if (phase === "disabled" || phase === "ready") {
    return <>{children}</>;
  }

  const retry = () => {
    if (phase === "logoutPending") {
      const client = clientRef.current;
      if (!client || logoutRetryInProgress.current) return;
      logoutRetryInProgress.current = true;
      setLogoutRetrying(true);
      void client
        .logout()
        .catch((cause: unknown) => {
          setError(describeAuthError(cause));
          setPhase("logoutPending");
        })
        .finally(() => {
          logoutRetryInProgress.current = false;
          setLogoutRetrying(false);
        });
      return;
    }
    const client = clientRef.current;
    if (!client) {
      try {
        clearRedirectAttempts(window.sessionStorage);
      } catch {
        // The initialization attempt will report a retryable storage error.
      }
      authRecoveryPending.current = false;
      setError(null);
      setPhase("checking");
      setInitializationAttempt((attempt) => attempt + 1);
      return;
    }
    // 明示的な再試行なので、自動リダイレクトの試行カウンタも仕切り直す。
    try {
      clearRedirectAttempts(window.sessionStorage);
    } catch {
      // startLogin will show a retryable error if storage is still unavailable.
    }
    setError(null);
    setPhase("checking");
    authRecoveryPending.current = false;
    startLogin(client, "/");
  };

  if (phase === "checking") return <ShellSkeleton />;

  return (
    <div className="flex h-dvh w-full items-center justify-center bg-background p-6 text-foreground">
      <div className="flex max-w-md flex-col items-center gap-4 text-center">
        <LanguageToggle />
        {phase === "loggedOut" ? (
          <>
            <h1 className="text-lg font-semibold">{t("auth.loggedOut")}</h1>
            <p className="text-sm text-muted-foreground">{t("auth.loggedOutHelp")}</p>
            <Button onClick={retry}>{t("auth.login")}</Button>
          </>
        ) : phase === "logoutPending" ? (
          <>
            <h1 className="text-lg font-semibold">{t("auth.logoutStorageFailed")}</h1>
            <p className="text-sm text-muted-foreground">{t("auth.logoutStorageFailedHelp")}</p>
            <Button onClick={retry} disabled={logoutRetrying}>
              {t("auth.retryLogout")}
            </Button>
          </>
        ) : (
          <>
            <h1 className="text-lg font-semibold">{t("auth.failed")}</h1>
            <p className="text-sm break-words text-muted-foreground">{formatAuthError(error, t)}</p>
            <Button onClick={retry}>{t("auth.retry")}</Button>
          </>
        )}
      </div>
    </div>
  );
}
