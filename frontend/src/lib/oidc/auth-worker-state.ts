import { audienceIncludes, decodeJwtPayload, issuerMatches } from "./jwt";
import type {
  AuthTokenResponse,
  AuthWorkerRequest,
  AuthWorkerResult,
  RefreshRequest,
} from "./auth-worker-protocol";

export interface AuthWorkerSession {
  binding: string;
  // pendingを送信前に保存する。ワーカー終了で応答不明なら再送しない。
  state: "ready" | "pending" | "ended";
  body?: AuthTokenResponse;
  expiresAt?: number;
  lastUsedAt?: number;
}

export interface AuthWorkerStore {
  get(sessionId: string): Promise<AuthWorkerSession | undefined>;
  put(sessionId: string, session: AuthWorkerSession): Promise<void>;
  staleSessionIds(cutoff: number): Promise<string[]>;
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

export const AUTH_WORKER_IDLE_MS = 24 * 60 * 60 * 1000;
const CLEANUP_INTERVAL_MS = 60 * 60 * 1000;

/** 全タブの更新を1つの所有先へ集約し、最新の結果を共有する。 */
export class AuthWorkerState {
  private nextCleanupAt = 0;
  private cleanupPromise: Promise<void> | null = null;

  constructor(private readonly environment: AuthWorkerEnvironment) {}

  async run(request: AuthWorkerRequest): Promise<AuthWorkerResult | null> {
    if (request.kind === "refresh") await this.cleanup();
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
      if (saved && saved.state !== "ready") throw new AuthWorkerError("session_ended");
      if (saved && saved.binding !== binding) throw new AuthWorkerError("session_mismatch");
      if (saved?.body && saved.expiresAt! > this.environment.now() + 60_000) {
        saved.lastUsedAt = this.environment.now();
        await store.put(request.sessionId, saved);
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
        lastUsedAt: this.environment.now(),
      };
      await store.put(request.sessionId, next);
      return this.responseWithRemainingLifetime(next);
    });
  }

  private async cleanup(): Promise<void> {
    if (this.cleanupPromise) return this.cleanupPromise;
    if (this.environment.now() < this.nextCleanupAt) return;
    const cutoff = this.environment.now() - AUTH_WORKER_IDLE_MS;
    const clean = async () => {
      for (const id of await this.environment.store.staleSessionIds(cutoff)) {
        await this.environment.exclusive(id, async () => {
          const current = await this.environment.store.get(id);
          // 他のWorkerが更新した行を消さない。終了記録は古いタブの復活防止に残す。
          if (current?.state === "ready" && (current.lastUsedAt ?? 0) <= cutoff) {
            await this.environment.store.put(id, { binding: current.binding, state: "ended" });
          }
        });
      }
      this.nextCleanupAt = this.environment.now() + CLEANUP_INTERVAL_MS;
    };
    this.cleanupPromise = clean();
    try {
      await this.cleanupPromise;
    } finally {
      this.cleanupPromise = null;
    }
  }

  private responseWithRemainingLifetime(session: AuthWorkerSession): AuthWorkerResult {
    return {
      ...session.body!,
      expires_in: Math.max(0, (session.expiresAt! - this.environment.now()) / 1000),
      expiresAt: session.expiresAt!,
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
      (body.expires_in !== undefined &&
        (typeof body.expires_in !== "number" ||
          !Number.isFinite(body.expires_in) ||
          body.expires_in <= 0)) ||
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
    return {
      ...body,
      expires_in: body.expires_in ?? 300,
      refresh_token: body.refresh_token ?? refreshToken,
    } as AuthTokenResponse;
  }
}
