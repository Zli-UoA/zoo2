import { describe, expect, it, vi } from "vite-plus/test";

import { OidcClient, type OidcEnvironment } from "./client";
import type { AuthStorage } from "./pending";
import { base64UrlEncode } from "./pkce";

const ISSUER = "https://account.zli.works/api/auth";
const CLIENT_ID = "zoo";
const AUTHZ_ENDPOINT = "https://account.zli.works/api/auth/oauth2/authorize";
const TOKEN_ENDPOINT = "https://account.zli.works/api/auth/oauth2/token";
const REDIRECT = "https://zli.chat/auth/reauth-callback.html";

function jsonResponse(body: unknown): Response {
  return new Response(JSON.stringify(body), { headers: { "Content-Type": "application/json" } });
}

function idToken(nonce: string): string {
  const encode = (value: unknown) =>
    base64UrlEncode(new TextEncoder().encode(JSON.stringify(value)));
  return `${encode({ alg: "ES256" })}.${encode({ iss: ISSUER, aud: CLIENT_ID, exp: 9_999_999_999, nonce })}.sig`;
}

function harness() {
  const items = new Map<string, string>();
  const storage: AuthStorage = {
    getItem: (key) => items.get(key) ?? null,
    setItem: (key, value) => void items.set(key, value),
    removeItem: (key) => void items.delete(key),
  };
  let nonce = "";
  const tokenBodies: URLSearchParams[] = [];
  const fetch = vi.fn(async (input: string, init?: RequestInit) => {
    if (input.endsWith("/.well-known/openid-configuration")) {
      return jsonResponse({
        issuer: ISSUER,
        authorization_endpoint: AUTHZ_ENDPOINT,
        token_endpoint: TOKEN_ENDPOINT,
      });
    }
    tokenBodies.push(new URLSearchParams(init?.body as string));
    return jsonResponse({ access_token: "new-access", id_token: idToken(nonce), expires_in: 300 });
  });
  const assign = vi.fn();
  const environment: OidcEnvironment = {
    origin: "https://zli.chat",
    href: "https://zli.chat/settings?tab=encryption",
    assign,
    replaceUrl: vi.fn(),
    storage,
    fetch,
    now: () => 1_800_000_000_000,
  };
  const client = new OidcClient({ issuer: ISSUER, clientId: CLIENT_ID, environment });
  return { client, items, tokenBodies, setNonce: (value: string) => (nonce = value), assign };
}

describe("OidcClient の再認証（#360）", () => {
  it("max_age=0 の認可リクエストを作り、verifier をストレージにもページにも置かない", async () => {
    const { client, items, assign } = harness();
    const request = await client.createReauthRequest(REDIRECT);
    const url = new URL(request.url);
    expect(url.origin + url.pathname).toBe(AUTHZ_ENDPOINT);
    expect(url.searchParams.get("max_age")).toBe("0");
    expect(url.searchParams.get("redirect_uri")).toBe(REDIRECT);
    expect(url.searchParams.get("state")).toBe(request.state);
    expect(url.searchParams.get("code_challenge_method")).toBe("S256");
    expect(request.url).not.toContain(request.verifier);
    expect([...items.values()].join()).not.toContain(request.verifier);
    expect(assign).not.toHaveBeenCalled();
  });

  it("認可コードを ID トークンに換え、今のセッションは変えない", async () => {
    const { client, tokenBodies, setNonce } = harness();
    const request = await client.createReauthRequest(REDIRECT);
    setNonce(request.nonce);
    const token = await client.exchangeReauthCode(request, "code-1");
    expect(token).toBe(idToken(request.nonce));
    expect(tokenBodies[0]?.get("code_verifier")).toBe(request.verifier);
    expect(tokenBodies[0]?.get("redirect_uri")).toBe(REDIRECT);
    expect(client.hasSession).toBe(false);
  });

  it("nonce が違う ID トークンは受け付けない", async () => {
    const { client, setNonce } = harness();
    const request = await client.createReauthRequest(REDIRECT);
    setNonce("other");
    await expect(client.exchangeReauthCode(request, "code-1")).rejects.toMatchObject({
      code: "nonce_mismatch",
    });
  });
});
