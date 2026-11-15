import { audienceIncludes, decodeJwtPayload, issuerMatches } from "./jwt";
import type { AuthTokenResponse, AuthWorkerRequest, RefreshRequest } from "./auth-worker-protocol";

export interface AuthWorkerSession {
  binding: string;
  // pendingを送信前に保存する。ワーカー終了で応答不明なら再送しない。
  state: "ready" | "pending" | "ended";
  body?: AuthTokenResponse;
  expiresAt?: number;
}

export interface AuthWorkerStore {
  get(sessionId: string): Promise<AuthWorkerSession | undefined>;
  put(sessionId: string, session: AuthWorkerSession): Promise<void>;
}

export class AuthWorkerError extends Error {
  constructor(
    readonly code: string,
    readonly status?: number,
  ) {
    super(code);
  }
}

export interface AuthWorkerEnvironment {
  store: AuthWorkerStore;
  exclusive<T>(sessionId: string, action: () => Promise<T>): Promise<T>;
  fetch(input: string, init: RequestInit): Promise<Response>;
  now(): number;
}

/** 全タブの更新を1つの所有先へ集約し、最新の結果を共有する。 */
export class AuthWorkerState {
  constructor(private readonly environment: AuthWorkerEnvironment) {}

  run(request: AuthWorkerRequest): Promise<AuthTokenResponse | null> {
    return this.environment.exclusive(request.sessionId, async () => {
      const { store } = this.environment;
      const saved = await store.get(request.sessionId);
      if (request.kind === "logout") {
        await store.put(request.sessionId, { binding: saved?.binding ?? "", state: "ended" });
        return null;
      }
      const binding = JSON.stringify([
        request.issuer,
        request.tokenEndpoint,
        request.clientId,
        request.resource,
      ]);
      if (saved && saved.binding !== binding) throw new AuthWorkerError("session_mismatch");
      if (saved && saved.state !== "ready") throw new AuthWorkerError("session_ended");
      if (saved?.body && saved.expiresAt! > this.environment.now() + 60_000) {
        const cached = this.responseWithRemainingLifetime(saved);
        // キャッシュ内のID tokenはすでに期限切れの可能性がある。
        delete cached.id_token;
        return cached;
      }
      // タブ側が古い値でも、ワーカーに保存した最新の更新トークンだけを使う。
      const refreshToken = saved?.body?.refresh_token ?? request.refreshToken;
      await store.put(request.sessionId, { binding, state: "pending" });
      const body = await this.exchange(request, refreshToken);
      const next: AuthWorkerSession = {
        binding,
        state: "ready",
        body,
        expiresAt: this.environment.now() + body.expires_in * 1000,
      };
      await store.put(request.sessionId, next);
      return this.responseWithRemainingLifetime(next);
    });
  }

  private responseWithRemainingLifetime(session: AuthWorkerSession): AuthTokenResponse {
    return {
      ...session.body!,
      expires_in: Math.max(0, (session.expiresAt! - this.environment.now()) / 1000),
    };
  }

  private async exchange(
    request: RefreshRequest,
    refreshToken: string,
  ): Promise<AuthTokenResponse> {
    const form = new URLSearchParams({
      grant_type: "refresh_token",
      refresh_token: refreshToken,
      client_id: request.clientId,
    });
    if (request.resource) form.set("resource", request.resource);
    const response = await this.environment.fetch(request.tokenEndpoint, {
      method: "POST",
      headers: { "Content-Type": "application/x-www-form-urlencoded", Accept: "application/json" },
      body: form.toString(),
      signal: AbortSignal.timeout(10_000),
    });
    const body = (await response.json()) as Record<string, unknown> | null;
    if (!response.ok)
      throw new AuthWorkerError(
        typeof body?.error === "string" ? body.error : "token_request_failed",
        response.status,
      );
    if (
      !body ||
      typeof body.access_token !== "string" ||
      !body.access_token ||
      typeof body.expires_in !== "number" ||
      !Number.isFinite(body.expires_in) ||
      body.expires_in <= 0 ||
      (body.refresh_token !== undefined &&
        (typeof body.refresh_token !== "string" || !body.refresh_token))
    ) {
      throw new AuthWorkerError("invalid_token_response");
    }
    // 共有キャッシュへ入れる前に、ページ側と同じID tokenの取り違え検査を行う。
    if (typeof body.id_token === "string") {
      const claims = decodeJwtPayload(body.id_token);
      if (
        !claims ||
        !issuerMatches(claims.iss, request.issuer) ||
        !audienceIncludes(claims.aud, request.clientId) ||
        typeof claims.exp !== "number" ||
        claims.exp * 1000 <= this.environment.now() - 60_000
      ) {
        throw new AuthWorkerError("invalid_id_token");
      }
    }
    return { ...body, refresh_token: body.refresh_token ?? refreshToken } as AuthTokenResponse;
  }
}
