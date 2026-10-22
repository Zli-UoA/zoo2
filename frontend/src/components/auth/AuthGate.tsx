import { useTranslation } from "react-i18next";
import { useRouter } from "@tanstack/react-router";
import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";

import { setAuthorizedHandler, setTokenProvider, setUnauthorizedHandler } from "../../lib/auth";
import { resolveConfig, type AppConfig } from "../../lib/config";
import { OidcClient } from "../../lib/oidc/client";
import {
  MAX_REDIRECT_ATTEMPTS,
  clearAuthorizeRedirectPending,
  clearLoggedOut,
  clearRedirectAttempts,
  isLoggedOut,
  markAuthorizeRedirectPending,
  markRedirectStarted,
  readUnauthorizedRedirectState,
  recordRedirectAttempt,
} from "../../lib/oidc/pending";
import { getOidcClient } from "../../lib/oidc/session";
import { LanguageToggle } from "../i18n/LanguageToggle";
import { ShellSkeleton } from "../layout/ShellSkeleton";
import { Button } from "../ui/button";

import { describeAuthError, formatAuthError, type AuthError } from "./authError";

type AuthPhase = "checking" | "ready" | "disabled" | "loggedOut" | "error";

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
 * 認証の入口。OIDC 未設定（dev の X-Dev-User）なら素通しし、設定済みなら
 * authorize リダイレクト / コールバック処理 / 401 時の再ログインを担う。
 *
 * トークンはメモリ保持のため、リロード後は IdP セッションを利用して
 * authorize へ再リダイレクトする（サイレント再ログイン）。
 */
export function AuthGate({ children }: { children: ReactNode }) {
  const { t } = useTranslation();
  const router = useRouter();
  const [phase, setPhase] = useState<AuthPhase>("checking");
  const [error, setError] = useState<AuthError | null>(null);
  const clientRef = useRef<OidcClient | null>(null);

  const startLogin = useCallback((client: OidcClient, returnTo: string) => {
    const storage = window.sessionStorage;
    const now = Date.now();
    markRedirectStarted(storage, now);
    markAuthorizeRedirectPending(storage, now);
    void client.login(returnTo).catch((cause: unknown) => {
      clearAuthorizeRedirectPending(storage);
      setError(describeAuthError(cause));
      setPhase("error");
    });
  }, []);

  useEffect(() => {
    // The parent exchanges the code and validates state/nonce. Never start login
    // or consume the interactive transaction inside the renewal iframe.
    if (window.parent !== window) {
      window.parent.postMessage(
        { type: "zoo:oidc:silent", url: window.location.href },
        window.location.origin,
      );
      return;
    }
    let cancelled = false;
    let config: AppConfig;
    try {
      config = resolveConfig();
    } catch (cause) {
      setError(describeAuthError(cause));
      setPhase("error");
      return;
    }

    const client = getOidcClient(config);
    if (!client) {
      setPhase("disabled");
      return;
    }
    clientRef.current = client;
    setTokenProvider(() => client.getAccessToken());
    client.setReauthorizeHandler((returnTo) => {
      startLogin(client, returnTo);
    });
    setAuthorizedHandler(() => {
      // バックエンドがトークンを受け付けたら試行カウンタを戻す。
      clearRedirectAttempts(window.sessionStorage);
    });
    setUnauthorizedHandler(() => {
      const storage = window.sessionStorage;
      const now = Date.now();
      const redirectState = readUnauthorizedRedirectState(storage, now, REDIRECT_COOLDOWN_MS);
      // 事前再認可や 401 起点のリダイレクト中に届いた 401 は、遷移前の
      // リクエストの結果。設定不一致のループ判定には使わない。
      if (redirectState === "redirecting") {
        return;
      }
      // リダイレクト直後の 401 は「IdP では通るのにバックエンドが拒否する」
      // 設定不一致の典型。再ログインせずエラー画面へ倒してループを止める。
      if (redirectState === "recent") {
        setError({ kind: "unauthorizedLoop" });
        setPhase("error");
        return;
      }
      clearLoggedOut(storage);
      client.resetSession();
      const attempts = recordRedirectAttempt(storage, now);
      if (attempts > MAX_REDIRECT_ATTEMPTS) {
        setError({ kind: "unauthorizedLoop" });
        setPhase("error");
        return;
      }
      startLogin(client, currentReturnTo());
    });

    // コールバック処理へ入る時点で進行中フラグを落とす。以後の 401 は
    // 設定不一致として扱える。通常ページの再読み込みで残った値もここで消す。
    clearAuthorizeRedirectPending(window.sessionStorage);

    // bfcache から戻った場合は mount されないため、進行中フラグを明示的に
    // 落とす。戻った先で 401 が続いても再ログインを止めないようにする。
    const handlePageShow = (event: PageTransitionEvent) => {
      if (event.persisted) {
        clearAuthorizeRedirectPending(window.sessionStorage);
      }
    };
    window.addEventListener("pageshow", handlePageShow);

    if (client.isCallbackUrl()) {
      client
        .handleCallback()
        .then(async ({ returnTo }) => {
          if (cancelled) {
            return;
          }
          clearLoggedOut(window.sessionStorage);
          router.history.replace(returnTo);
          // 復帰先のルート解決を待ってから描画する（コールバック URL の
          // notFound が一瞬描画されるのを防ぐ）。
          await router.load().catch(() => {});
          if (cancelled) {
            return;
          }
          setPhase("ready");
        })
        .catch((cause: unknown) => {
          if (cancelled) {
            return;
          }
          // code / error を含むコールバック URL を残すと、リロードのたびに
          // 同じ失敗を繰り返すため、エラー画面へ移る前に置き換える。
          router.history.replace("/");
          void router.load().catch(() => {});
          setError(describeAuthError(cause));
          setPhase("error");
        });
    } else if (client.hasSession) {
      setPhase("ready");
    } else if (isLoggedOut(window.sessionStorage)) {
      setPhase("loggedOut");
    } else {
      startLogin(client, currentReturnTo());
    }

    return () => {
      cancelled = true;
      window.removeEventListener("pageshow", handlePageShow);
      setUnauthorizedHandler(null);
      setAuthorizedHandler(null);
      client.setReauthorizeHandler(null);
    };
  }, [router, startLogin]);

  if (phase === "disabled" || phase === "ready") {
    return <>{children}</>;
  }

  if (phase === "checking") {
    return <ShellSkeleton />;
  }

  const retry = () => {
    const client = clientRef.current;
    if (!client) {
      return;
    }
    clearLoggedOut(window.sessionStorage);
    // 明示的な再試行なので、自動リダイレクトの試行カウンタも仕切り直す。
    clearRedirectAttempts(window.sessionStorage);
    setError(null);
    setPhase("checking");
    startLogin(client, "/");
  };

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
