import { afterAll, beforeAll, describe, expect, it } from "vite-plus/test";

import { OidcClient, type OidcEnvironment } from "./client";
import { startMockIdp, type MockIdp } from "../../../e2e/mock-idp.ts";
import { takePending, type AuthStorage } from "./pending";

function memoryStorage(): AuthStorage {
  const map = new Map<string, string>();
  return {
    getItem: (key) => map.get(key) ?? null,
    setItem: (key, value) => void map.set(key, value),
    removeItem: (key) => void map.delete(key),
  };
}

interface Browser {
  env: OidcEnvironment;
  assigned: string[];
  setHref(href: string): void;
  advance(ms: number): void;
}

function createBrowser(): Browser {
  const assigned: string[] = [];
  let clock = Date.now();
  let href = "https://app.example/";
  const storage = memoryStorage();
  const env: OidcEnvironment = {
    origin: "https://app.example",
    get href() {
      return href;
    },
    assign: (url) => void assigned.push(url),
    replaceUrl: (url) => void (href = url),
    storage,
    fetch: (input, init) => fetch(input, init),
    now: () => clock,
  };
  return {
    env,
    assigned,
    setHref: (value) => {
      href = value;
    },
    advance: (ms) => {
      clock += ms;
    },
  };
}

/** authorize 画面（モック IdP の自動ログイン）を経てコールバック URL を得る。 */
async function authorize(idp: MockIdp, client: OidcClient): Promise<string> {
  const authorizeUrl = await client.login("/channels/abc");
  const url = new URL(authorizeUrl);
  expect(url.origin).toBe(idp.origin);
  url.searchParams.set("auto", "1");
  const response = await fetch(url, { redirect: "manual" });
  expect(response.status).toBe(302);
  const location = response.headers.get("location");
  if (!location) {
    throw new Error("mock IdP did not redirect back to the callback");
  }
  return location;
}

describe("OIDC login against a mock IdP", () => {
  let idp: MockIdp;

  beforeAll(async () => {
    idp = await startMockIdp({ accessTokenTtlSeconds: 120 });
  });

  afterAll(async () => {
    await idp.close();
  });

  it("completes login → Bearer → refresh → logout", async () => {
    const browser = createBrowser();
    const client = new OidcClient({
      issuer: idp.issuer,
      clientId: idp.clientId,
      environment: browser.env,
    });

    const callbackUrl = await authorize(idp, client);
    browser.setHref(callbackUrl);
    const result = await client.handleCallback();
    expect(result.returnTo).toBe("/channels/abc");

    const accessToken = await client.getAccessToken();
    expect(accessToken).toBeTruthy();

    const verified = idp.verifyAccessToken(accessToken!);
    expect(verified.valid).toBe(true);
    expect(verified.header.typ).toBe("at+jwt");
    expect(verified.header.alg).toBe("ES256");
    expect(verified.claims.iss).toBe(idp.issuer);
    expect(verified.claims.aud).toBe(idp.clientId);
    expect(verified.claims.sub).toBeTruthy();

    // 期限に近づけると refresh_token で自動更新される。
    const firstRefreshToken = idp.issuedRefreshTokens[0];
    browser.advance(70_000);
    const refreshed = await client.getAccessToken();
    expect(refreshed).toBeTruthy();
    expect(refreshed).not.toBe(accessToken);
    expect(idp.issuedRefreshTokens).toHaveLength(2);

    // Kanidm 同様、使用済みの refresh_token は拒否される。
    const reuse = await fetch(idp.tokenEndpoint, {
      method: "POST",
      headers: { "Content-Type": "application/x-www-form-urlencoded" },
      body: new URLSearchParams({
        grant_type: "refresh_token",
        refresh_token: firstRefreshToken!,
        client_id: idp.clientId,
      }).toString(),
    });
    expect(reuse.status).toBe(400);

    await client.logout();
    expect(client.hasSession).toBe(false);
    const logoutUrl = new URL(browser.assigned.at(-1)!);
    expect(logoutUrl.origin + logoutUrl.pathname).toBe(idp.endSessionEndpoint);
    expect(logoutUrl.searchParams.get("post_logout_redirect_uri")).toBe("https://app.example/");
    expect(logoutUrl.searchParams.get("state")).toBeTruthy();
  });

  it("rejects a token exchange with a wrong code_verifier", async () => {
    const browser = createBrowser();
    const client = new OidcClient({
      issuer: idp.issuer,
      clientId: idp.clientId,
      environment: browser.env,
    });

    const callbackUrl = await authorize(idp, client);
    const callback = new URL(callbackUrl);
    const code = callback.searchParams.get("code")!;
    expect(callback.searchParams.get("state")).toBe(
      takePending(browser.env.storage, browser.env.now())?.state,
    );

    const response = await fetch(idp.tokenEndpoint, {
      method: "POST",
      headers: { "Content-Type": "application/x-www-form-urlencoded" },
      body: new URLSearchParams({
        grant_type: "authorization_code",
        code,
        redirect_uri: "https://app.example/auth/callback",
        client_id: idp.clientId,
        code_verifier: "wrong-verifier",
      }).toString(),
    });
    expect(response.status).toBe(400);
    await expect(response.json()).resolves.toMatchObject({ error: "invalid_grant" });
  });

  it("rejects a callback whose state does not match the stored request", async () => {
    const browser = createBrowser();
    const client = new OidcClient({
      issuer: idp.issuer,
      clientId: idp.clientId,
      environment: browser.env,
    });

    await client.login();
    browser.setHref("https://app.example/auth/callback?code=c&state=tampered");
    await expect(client.handleCallback()).rejects.toMatchObject({ code: "state_mismatch" });
  });
});
