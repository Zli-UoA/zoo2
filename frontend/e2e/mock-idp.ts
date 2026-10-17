import {
  createHash,
  generateKeyPairSync,
  randomUUID,
  sign as signWithKey,
  verify as verifyWithKey,
} from "node:crypto";
import { createServer, type IncomingMessage, type Server, type ServerResponse } from "node:http";
import type { AddressInfo } from "node:net";

export interface MockIdpUser {
  sub: string;
  preferredUsername: string;
  name: string;
  email: string;
}

export interface MockIdpOptions {
  clientId?: string;
  user?: Partial<MockIdpUser>;
  /** アクセストークンの有効期限（秒）。 */
  accessTokenTtlSeconds?: number;
  /** 付与する scope。要求 scope に関わらずこれを返す。 */
  scope?: string;
  /** 待ち受けるポート。未指定なら空きポートを自動で使う。 */
  port?: number;
}

export interface MockIdpTokens {
  access_token: string;
  token_type: "Bearer";
  expires_in: number;
  refresh_token: string;
  id_token: string;
  scope: string;
}

export interface MockIdp {
  issuer: string;
  clientId: string;
  authorizationEndpoint: string;
  tokenEndpoint: string;
  endSessionEndpoint: string;
  jwksUri: string;
  /** 直近に発行したアクセストークン。 */
  readonly lastAccessToken: string | null;
  /** 発行済み refresh_token のローテーション確認用。 */
  readonly issuedRefreshTokens: string[];
  /** テストから sub を差し替える（JIT の再ログイン確認用）。 */
  setUser(user: Partial<MockIdpUser>): void;
  /** 署名と typ を検証する。Rust の検証仕様（at+jwt / ES256）に合わせている。 */
  verifyAccessToken(token: string): {
    valid: boolean;
    header: Record<string, unknown>;
    claims: Record<string, unknown>;
  };
  origin: string;
  close(): Promise<void>;
}

const BASE64URL = (value: Buffer | Uint8Array): string => Buffer.from(value).toString("base64url");

function decodePart(part: string): Record<string, unknown> {
  return JSON.parse(Buffer.from(part, "base64url").toString("utf8")) as Record<string, unknown>;
}

function escapeHtml(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function renderLoginPage(action: string, params: URLSearchParams): string {
  const hidden = [...params.entries()]
    .map(
      ([key, value]) =>
        `<input type="hidden" name="${escapeHtml(key)}" value="${escapeHtml(value)}" />`,
    )
    .join("\n");
  return `<!doctype html>
<html lang="ja">
  <head><meta charset="utf-8" /><title>Mock IdP ログイン</title></head>
  <body style="font-family: sans-serif; display: grid; place-items: center; height: 100vh; margin: 0">
    <form method="get" action="${escapeHtml(action)}">
      <h1>Mock IdP</h1>
      <p>zoo へのログインを許可しますか？</p>
      ${hidden}
      <button type="submit" name="auto" value="1">ログイン</button>
    </form>
  </body>
</html>`;
}

function sendJson(response: ServerResponse, status: number, body: unknown): void {
  const payload = JSON.stringify(body);
  response.writeHead(status, {
    "Content-Type": "application/json",
    "Cache-Control": "no-store",
  });
  response.end(payload);
}

async function readBody(request: IncomingMessage): Promise<string> {
  const chunks: Buffer[] = [];
  for await (const chunk of request) {
    chunks.push(Buffer.from(chunk));
  }
  return Buffer.concat(chunks).toString("utf8");
}

/**
 * OIDC Authorization Code + PKCE の極小モック IdP。
 *
 * 外部依存なしの Node http サーバで、Rust の検証仕様（`typ: at+jwt` / ES256）に
 * 合わせたアクセストークンを署名する。Kanidm の refresh_token ローテーション
 * （使い捨て）も模し、再利用は invalid_grant で拒否する。
 */
export async function startMockIdp(options: MockIdpOptions = {}): Promise<MockIdp> {
  const clientId = options.clientId ?? "zoo";
  const accessTtl = options.accessTokenTtlSeconds ?? 300;
  const scope = options.scope ?? "openid profile email";
  const kid = "mock-idp-key-1";
  const { privateKey, publicKey } = generateKeyPairSync("ec", { namedCurve: "prime256v1" });
  const publicJwk = {
    ...(publicKey.export({ format: "jwk" }) as Record<string, unknown>),
    kid,
    alg: "ES256",
    use: "sig",
  };

  const user: MockIdpUser = {
    sub: options.user?.sub ?? randomUUID(),
    preferredUsername: options.user?.preferredUsername ?? "smoke",
    name: options.user?.name ?? "Smoke User",
    email: options.user?.email ?? "smoke@example.com",
  };

  interface AuthorizationCode {
    challenge: string;
    redirectUri: string;
    nonce: string;
    used: boolean;
  }

  const codes = new Map<string, AuthorizationCode>();
  const refreshTokens = new Map<string, { sub: string }>();
  const issuedRefreshTokens: string[] = [];
  let lastAccessToken: string | null = null;

  let issuer = "";
  let origin = "";

  const signJwt = (header: Record<string, unknown>, payload: Record<string, unknown>): string => {
    const encodedHeader = BASE64URL(Buffer.from(JSON.stringify(header)));
    const encodedPayload = BASE64URL(Buffer.from(JSON.stringify(payload)));
    const signingInput = `${encodedHeader}.${encodedPayload}`;
    const signature = signWithKey("sha256", Buffer.from(signingInput), {
      key: privateKey,
      dsaEncoding: "ieee-p1363",
    });
    return `${signingInput}.${BASE64URL(signature)}`;
  };

  const nowSeconds = () => Math.floor(Date.now() / 1000);

  const issueTokens = (sub: string, nonce: string): MockIdpTokens => {
    const accessToken = signJwt(
      { alg: "ES256", typ: "at+jwt", kid },
      {
        iss: issuer,
        sub,
        aud: clientId,
        exp: nowSeconds() + accessTtl,
        iat: nowSeconds(),
        nbf: nowSeconds(),
        scope,
        preferred_username: user.preferredUsername,
        name: user.name,
        email: user.email,
      },
    );
    lastAccessToken = accessToken;
    const refreshToken = randomUUID();
    refreshTokens.set(refreshToken, { sub });
    issuedRefreshTokens.push(refreshToken);
    const idToken = signJwt(
      { alg: "ES256", typ: "JWT", kid },
      {
        iss: issuer,
        sub,
        aud: clientId,
        exp: nowSeconds() + 300,
        iat: nowSeconds(),
        nonce,
      },
    );
    return {
      access_token: accessToken,
      token_type: "Bearer",
      expires_in: accessTtl,
      refresh_token: refreshToken,
      id_token: idToken,
      scope,
    };
  };

  const server: Server = createServer((request, response) => {
    void handle(request, response);
  });

  async function handle(request: IncomingMessage, response: ServerResponse): Promise<void> {
    const url = new URL(request.url ?? "/", origin);
    const path = url.pathname.slice(`/oauth2/openid/${clientId}`.length);

    // SPA は別オリジンから Discovery / token を叩くため CORS を許可する
    // （Kanidm 実接続でも同等の許可が要る。詳細は infra/README.md）。
    response.setHeader("Access-Control-Allow-Origin", "*");
    response.setHeader("Access-Control-Allow-Methods", "GET, POST, OPTIONS");
    response.setHeader("Access-Control-Allow-Headers", "Content-Type, Authorization, Accept");
    if (request.method === "OPTIONS") {
      response.writeHead(204);
      response.end();
      return;
    }

    // テスト専用: E2E から sub を切り替えて別ユーザーとしてログインし直す。
    if (url.pathname === "/__mock/user" && request.method === "POST") {
      let patch: Partial<MockIdpUser>;
      try {
        patch = JSON.parse(await readBody(request)) as Partial<MockIdpUser>;
      } catch {
        sendJson(response, 400, { error: "invalid_json" });
        return;
      }
      if (typeof patch.sub === "string") user.sub = patch.sub;
      if (typeof patch.preferredUsername === "string")
        user.preferredUsername = patch.preferredUsername;
      if (typeof patch.name === "string") user.name = patch.name;
      if (typeof patch.email === "string") user.email = patch.email;
      sendJson(response, 200, { sub: user.sub });
      return;
    }

    if (path === "/.well-known/openid-configuration") {
      sendJson(response, 200, {
        issuer,
        authorization_endpoint: `${issuer}/authorize`,
        token_endpoint: `${issuer}/token`,
        jwks_uri: `${issuer}/jwks`,
        end_session_endpoint: `${issuer}/logout`,
        response_types_supported: ["code"],
        code_challenge_methods_supported: ["S256"],
        id_token_signing_alg_values_supported: ["ES256"],
        token_endpoint_auth_methods_supported: ["none"],
      });
      return;
    }

    if (path === "/jwks") {
      sendJson(response, 200, { keys: [publicJwk] });
      return;
    }

    if (path === "/authorize" && request.method === "GET") {
      const redirectUri = url.searchParams.get("redirect_uri");
      const state = url.searchParams.get("state");
      const challenge = url.searchParams.get("code_challenge");
      const method = url.searchParams.get("code_challenge_method");
      const nonce = url.searchParams.get("nonce") ?? "";
      if (
        !redirectUri ||
        !state ||
        !challenge ||
        method !== "S256" ||
        url.searchParams.get("client_id") !== clientId
      ) {
        sendJson(response, 400, { error: "invalid_request" });
        return;
      }
      if (url.searchParams.get("auto") !== "1") {
        response.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
        response.end(renderLoginPage(url.pathname, url.searchParams));
        return;
      }
      const code = randomUUID();
      codes.set(code, { challenge, redirectUri, nonce, used: false });
      const location = new URL(redirectUri);
      location.searchParams.set("code", code);
      location.searchParams.set("state", state);
      response.writeHead(302, { Location: location.toString() });
      response.end();
      return;
    }

    if (path === "/token" && request.method === "POST") {
      const body = new URLSearchParams(await readBody(request));
      const grantType = body.get("grant_type");
      if (body.get("client_id") !== clientId) {
        sendJson(response, 400, { error: "invalid_client" });
        return;
      }
      if (grantType === "authorization_code") {
        const code = body.get("code") ?? "";
        const stored = codes.get(code);
        if (!stored || stored.used) {
          sendJson(response, 400, { error: "invalid_grant" });
          return;
        }
        stored.used = true;
        if (body.get("redirect_uri") !== stored.redirectUri) {
          sendJson(response, 400, { error: "invalid_grant" });
          return;
        }
        const verifier = body.get("code_verifier") ?? "";
        const challenge = BASE64URL(createHash("sha256").update(verifier).digest());
        if (challenge !== stored.challenge) {
          sendJson(response, 400, { error: "invalid_grant" });
          return;
        }
        sendJson(response, 200, issueTokens(user.sub, stored.nonce));
        return;
      }
      if (grantType === "refresh_token") {
        const token = body.get("refresh_token") ?? "";
        const stored = refreshTokens.get(token);
        if (!stored) {
          sendJson(response, 400, { error: "invalid_grant" });
          return;
        }
        // Kanidm 同様、refresh_token は使い捨て。使ったら無効化してローテーションする。
        refreshTokens.delete(token);
        sendJson(response, 200, issueTokens(stored.sub, ""));
        return;
      }
      sendJson(response, 400, { error: "unsupported_grant_type" });
      return;
    }

    if (path === "/logout" && request.method === "GET") {
      const postLogout = url.searchParams.get("post_logout_redirect_uri") ?? "/";
      response.writeHead(302, { Location: postLogout });
      response.end();
      return;
    }

    sendJson(response, 404, { error: "not_found" });
  }

  await new Promise<void>((resolve) => {
    server.listen(options.port ?? 0, "127.0.0.1", () => resolve());
  });
  const address = server.address() as AddressInfo;
  origin = `http://127.0.0.1:${address.port}`;
  issuer = `${origin}/oauth2/openid/${clientId}`;

  return {
    issuer,
    origin,
    clientId,
    authorizationEndpoint: `${issuer}/authorize`,
    tokenEndpoint: `${issuer}/token`,
    endSessionEndpoint: `${issuer}/logout`,
    jwksUri: `${issuer}/jwks`,
    get lastAccessToken() {
      return lastAccessToken;
    },
    issuedRefreshTokens,
    setUser(next) {
      Object.assign(user, next);
    },
    verifyAccessToken(token) {
      const parts = token.split(".");
      if (parts.length !== 3) {
        return { valid: false, header: {}, claims: {} };
      }
      const header = decodePart(parts[0]!);
      const claims = decodePart(parts[1]!);
      const valid = verifyWithKey(
        "sha256",
        Buffer.from(`${parts[0]}.${parts[1]}`),
        { key: publicKey, dsaEncoding: "ieee-p1363" },
        Buffer.from(parts[2]!, "base64url"),
      );
      return { valid, header, claims };
    },
    async close() {
      await new Promise<void>((resolve, reject) => {
        server.close((error) => (error ? reject(error) : resolve()));
      });
    },
  };
}
