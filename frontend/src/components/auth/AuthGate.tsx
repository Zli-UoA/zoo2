import { useRouter } from "@tanstack/react-router";
import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";

import { setTokenProvider, setUnauthorizedHandler } from "../../lib/auth";
import { resolveConfig, type AppConfig } from "../../lib/config";
import { OidcClient, OidcError } from "../../lib/oidc/client";
import { clearLoggedOut, isLoggedOut } from "../../lib/oidc/pending";
import { getOidcClient } from "../../lib/oidc/session";
import { ShellSkeleton } from "../layout/ShellSkeleton";
import { Button } from "../ui/button";

type AuthPhase = "checking" | "ready" | "disabled" | "loggedOut" | "error";

/** 認証エラー時の再ログイン連打・リダイレクトループを防ぐクールダウン。 */
const REDIRECT_COOLDOWN_MS = 10_000;

let lastRedirectAt = 0;

function currentReturnTo(): string {
  if (typeof window === "undefined") {
    return "/";
  }
  const { pathname, search, hash } = window.location;
  return `${pathname}${search}${hash}`;
}

function describeError(error: unknown): string {
  if (error instanceof OidcError) {
    return `${error.message}（${error.code}）`;
  }
  if (error instanceof Error) {
    return error.message;
  }
  return String(error);
}

/**
 * 認証の入口。OIDC 未設定（dev の X-Dev-User）なら素通しし、設定済みなら
 * authorize リダイレクト / コールバック処理 / 401 時の再ログインを担う。
 *
 * トークンはメモリ保持のため、リロード後は IdP セッションを利用して
 * authorize へ再リダイレクトする（サイレント再ログイン）。
 */
export function AuthGate({ children }: { children: ReactNode }) {
  const router = useRouter();
  const [phase, setPhase] = useState<AuthPhase>("checking");
  const [error, setError] = useState<string | null>(null);
  const clientRef = useRef<OidcClient | null>(null);

  const startLogin = useCallback((client: OidcClient, returnTo: string) => {
    const now = Date.now();
    if (now - lastRedirectAt < REDIRECT_COOLDOWN_MS) {
      return;
    }
    lastRedirectAt = now;
    void client.login(returnTo).catch((cause: unknown) => {
      setError(describeError(cause));
      setPhase("error");
    });
  }, []);

  useEffect(() => {
    let cancelled = false;
    let config: AppConfig;
    try {
      config = resolveConfig();
    } catch (cause) {
      setError(describeError(cause));
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
    setUnauthorizedHandler(() => {
      if (Date.now() - lastRedirectAt < REDIRECT_COOLDOWN_MS) {
        return;
      }
      clearLoggedOut(window.sessionStorage);
      client.resetSession();
      startLogin(client, currentReturnTo());
    });

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
          setError(describeError(cause));
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
      setUnauthorizedHandler(null);
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
    lastRedirectAt = 0;
    setError(null);
    setPhase("checking");
    startLogin(client, "/");
  };

  return (
    <div className="flex h-dvh w-full items-center justify-center bg-background p-6 text-foreground">
      <div className="flex max-w-md flex-col items-center gap-4 text-center">
        {phase === "loggedOut" ? (
          <>
            <h1 className="text-lg font-semibold">ログアウトしました</h1>
            <p className="text-sm text-muted-foreground">
              Kanidm のセッションは残っているため、続けるには再度ログインしてください。
            </p>
            <Button onClick={retry}>ログイン</Button>
          </>
        ) : (
          <>
            <h1 className="text-lg font-semibold">ログインできませんでした</h1>
            <p className="text-sm break-words text-muted-foreground">{error}</p>
            <Button onClick={retry}>再ログイン</Button>
          </>
        )}
      </div>
    </div>
  );
}
