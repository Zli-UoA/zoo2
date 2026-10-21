import { describe, expect, it, vi } from "vite-plus/test";

import {
  DEFAULT_CALLBACK_PATH,
  OidcClient,
  OidcError,
  sanitizeReturnTo,
  type OidcEnvironment,
  type OidcFetch,
} from "./client";
import { savePending, takePending, type AuthStorage } from "./pending";
import { base64UrlEncode, createChallenge } from "./pkce";

const ISSUER = "https://id.example/oauth2/openid/zoo";
const CLIENT_ID = "zoo";
const AUTHZ_ENDPOINT = "https://id.example/ui/oauth2";
const TOKEN_ENDPOINT = "https://id.example/oauth2/token";
const END_SESSION_ENDPOINT = "https://id.example/ui/logout";
const CALLBACK_URL = "https://app.example/auth/callback";

function memoryStorage(): AuthStorage {
  const map = new Map<string, string>();
  return {
    getItem: (key) => map.get(key) ?? null,
    setItem: (key, value) => void map.set(key, value),
    removeItem: (key) => void map.delete(key),
  };
}

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

function discoveryBody(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    issuer: ISSUER,
    authorization_endpoint: AUTHZ_ENDPOINT,
    token_endpoint: TOKEN_ENDPOINT,
    end_session_endpoint: END_SESSION_ENDPOINT,
    ...overrides,
  };
}

function idToken(claims: {
  nonce?: string;
  iss?: string;
  aud?: string | string[];
  exp?: number;
}): string {
  const encode = (value: unknown) =>
    base64UrlEncode(new TextEncoder().encode(JSON.stringify(value)));
  return `${encode({ alg: "ES256", typ: "JWT" })}.${encode({
    iss: ISSUER,
    aud: CLIENT_ID,
    exp: 9_999_999_999,
    ...claims,
  })}.signature`;
}

interface Harness {
  client: OidcClient;
  storage: AuthStorage;
  assigned: string[];
  fetchMock: ReturnType<typeof vi.fn<OidcFetch>>;
  tokenCalls: URLSearchParams[];
  advance(ms: number): void;
  refreshCalls(): URLSearchParams[];
  /** token ハンドラが参照する nonce を、login 後に設定する。 */
  setNonce(nonce: string): void;
}

interface HarnessOptions {
  discovery?: Record<string, unknown>;
  /** token エンドポイントの応答。`URLSearchParams` と現在の nonce を受け取る。 */
  token?: (body: URLSearchParams, nonce: string) => unknown;
}

function createHarness(options: HarnessOptions = {}): Harness {
  const storage = memoryStorage();
  const assigned: string[] = [];
  const tokenCalls: URLSearchParams[] = [];
  let clock = 1_000_000;
  let lastNonce = "nonce";

  const fetchMock = vi.fn<OidcFetch>(async (input, init) => {
    if (input.endsWith("/.well-known/openid-configuration")) {
      return jsonResponse(options.discovery ?? discoveryBody());
    }
    const body = new URLSearchParams(init?.body as string);
    tokenCalls.push(body);
    if (!options.token) {
      return jsonResponse({
        access_token: "access-1",
        id_token: idToken({ nonce: lastNonce }),
        expires_in: 300,
      });
    }
    const result = await options.token(body, lastNonce);
    if (result instanceof Response) {
      return result;
    }
    const status =
      typeof result === "object" && result !== null && "status" in result
        ? Number((result as { status: unknown }).status)
        : 200;
    return jsonResponse(result, status);
  });

  const env: OidcEnvironment = {
    origin: "https://app.example",
    href: "https://app.example/",
    assign: (url) => void assigned.push(url),
    replaceUrl: () => {},
    storage,
    fetch: fetchMock,
    now: () => clock,
  };
  const client = new OidcClient({
    issuer: ISSUER,
    clientId: CLIENT_ID,
    environment: env,
  });

  return {
    client,
    storage,
    assigned,
    fetchMock,
    tokenCalls,
    advance: (ms) => {
      clock += ms;
    },
    refreshCalls: () => tokenCalls.filter((body) => body.get("grant_type") === "refresh_token"),
    setNonce: (value) => {
      lastNonce = value;
    },
  };
}

/** login → コールバック処理まで通し、保存した認可リクエストを返す。 */
async function completeLogin(harness: Harness) {
  await harness.client.login();
  const pending = takePending(harness.storage, 1_000_000 + 1);
  if (!pending) {
    throw new Error("pending was not stored");
  }
  savePending(harness.storage, pending);
  harness.setNonce(pending.nonce);
  await harness.client.handleCallback(`${CALLBACK_URL}?code=code-1&state=${pending.state}`);
  return pending;
}

describe("sanitizeReturnTo", () => {
  it("keeps same-origin relative paths only", () => {
    expect(sanitizeReturnTo("/channels/abc?x=1")).toBe("/channels/abc?x=1");
    expect(sanitizeReturnTo("/channels/abc#top")).toBe("/channels/abc#top");
    expect(sanitizeReturnTo("https://evil.example")).toBe("/");
    expect(sanitizeReturnTo("//evil.example")).toBe("/");
    expect(sanitizeReturnTo("/\\evil.example")).toBe("/");
    expect(sanitizeReturnTo("/channels\\abc")).toBe("/");
    expect(sanitizeReturnTo(undefined)).toBe("/");
    expect(sanitizeReturnTo(DEFAULT_CALLBACK_PATH)).toBe("/");
    expect(sanitizeReturnTo(`${DEFAULT_CALLBACK_PATH}?code=x`)).toBe("/");
    expect(sanitizeReturnTo(`${DEFAULT_CALLBACK_PATH}#x`)).toBe("/");
    expect(sanitizeReturnTo(`${DEFAULT_CALLBACK_PATH}/`)).toBe("/");
  });
});

describe("OidcClient discovery", () => {
  it("fetches and caches the discovery document", async () => {
    const harness = createHarness();
    await harness.client.discover();
    await harness.client.discover();

    const discoveryCalls = harness.fetchMock.mock.calls.filter(([input]) =>
      input.endsWith("/.well-known/openid-configuration"),
    );
    expect(discoveryCalls).toHaveLength(1);
    expect(discoveryCalls[0]?.[0]).toBe(`${ISSUER}/.well-known/openid-configuration`);
  });

  it("rejects an issuer mismatch", async () => {
    const harness = createHarness({ discovery: discoveryBody({ issuer: "https://evil.example" }) });
    await expect(harness.client.discover()).rejects.toMatchObject({ code: "issuer_mismatch" });
  });

  it("rejects a discovery document without required endpoints", async () => {
    const harness = createHarness({
      discovery: { issuer: ISSUER, authorization_endpoint: AUTHZ_ENDPOINT },
    });
    await expect(harness.client.discover()).rejects.toMatchObject({ code: "invalid_discovery" });
  });

  it("allows a missing end_session_endpoint", async () => {
    const harness = createHarness({
      discovery: discoveryBody({ end_session_endpoint: undefined }),
    });
    await expect(harness.client.discover()).resolves.toMatchObject({ endSessionEndpoint: null });
  });
});

describe("OidcClient.login", () => {
  it("builds an S256 authorize URL and stores the pending request", async () => {
    const harness = createHarness();
    const href = await harness.client.login("/channels/abc?tab=info");

    const url = new URL(href);
    expect(url.origin + url.pathname).toBe(AUTHZ_ENDPOINT);
    expect(url.searchParams.get("response_type")).toBe("code");
    expect(url.searchParams.get("client_id")).toBe(CLIENT_ID);
    expect(url.searchParams.get("redirect_uri")).toBe("https://app.example/auth/callback");
    expect(url.searchParams.get("scope")).toBe("openid profile");
    expect(url.searchParams.get("code_challenge_method")).toBe("S256");
    expect(harness.assigned).toEqual([href]);

    const pending = takePending(harness.storage, 1_000_000);
    expect(url.searchParams.get("state")).toBe(pending?.state);
    expect(url.searchParams.get("nonce")).toBe(pending?.nonce);
    expect(url.searchParams.get("code_challenge")).toBe(
      await createChallenge(pending?.verifier ?? ""),
    );
    expect(pending?.returnTo).toBe("/channels/abc?tab=info");
  });
});

describe("OidcClient.handleCallback", () => {
  it("exchanges the code with the PKCE verifier and stores tokens", async () => {
    const harness = createHarness();
    const { nonce } = await completeLogin(harness);

    await expect(harness.client.getAccessToken()).resolves.toBe("access-1");
    const body = harness.refreshCalls()[0] ?? harness.tokenCalls.at(-1);
    expect(body?.get("grant_type")).toBe("authorization_code");
    expect(body?.get("code")).toBe("code-1");
    expect(body?.get("client_id")).toBe(CLIENT_ID);
    expect(body?.get("redirect_uri")).toBe(CALLBACK_URL);
    expect(nonce.length).toBeGreaterThan(0);
    const pendingVerifier = body?.get("code_verifier");
    expect(pendingVerifier).toBeTruthy();
  });

  it("returns the sanitized returnTo", async () => {
    const harness = createHarness();
    await harness.client.login("/channels/abc");
    const pending = takePending(harness.storage, 1_000_000)!;
    savePending(harness.storage, pending);
    harness.setNonce(pending.nonce);
    const result = await harness.client.handleCallback(
      `${CALLBACK_URL}?code=code-1&state=${pending.state}`,
    );
    expect(result.returnTo).toBe("/channels/abc");
  });

  it("requires an id_token on the code exchange", async () => {
    const harness = createHarness({
      token: () => ({ access_token: "access-1", expires_in: 300 }),
    });
    await harness.client.login();
    const pending = takePending(harness.storage, 1_000_000)!;
    savePending(harness.storage, pending);
    await expect(
      harness.client.handleCallback(`${CALLBACK_URL}?code=c&state=${pending.state}`),
    ).rejects.toMatchObject({ code: "invalid_id_token" });
    expect(harness.client.hasSession).toBe(false);
  });

  it("processes the same callback URL only once", async () => {
    const harness = createHarness();
    await harness.client.login();
    const pending = takePending(harness.storage, 1_000_000)!;
    savePending(harness.storage, pending);
    harness.setNonce(pending.nonce);
    const callback = `${CALLBACK_URL}?code=code-1&state=${pending.state}`;

    const [first, second] = await Promise.all([
      harness.client.handleCallback(callback),
      harness.client.handleCallback(callback),
    ]);

    expect(first).toEqual(second);
    expect(harness.tokenCalls).toHaveLength(1);
  });

  it("rejects an authorize error response", async () => {
    const harness = createHarness();
    await expect(
      harness.client.handleCallback(`${CALLBACK_URL}?error=access_denied&error_description=denied`),
    ).rejects.toMatchObject({ code: "access_denied" });
  });

  it("rejects a callback without a stored pending request", async () => {
    const harness = createHarness();
    await expect(
      harness.client.handleCallback(`${CALLBACK_URL}?code=c&state=s`),
    ).rejects.toMatchObject({ code: "missing_pending_request" });
  });

  it("rejects a state mismatch", async () => {
    const harness = createHarness();
    await harness.client.login();
    await expect(
      harness.client.handleCallback(`${CALLBACK_URL}?code=c&state=other`),
    ).rejects.toMatchObject({ code: "state_mismatch" });
  });

  it("rejects a nonce mismatch without storing a session", async () => {
    const harness = createHarness({
      token: () => ({ access_token: "access-1", id_token: idToken({ nonce: "wrong" }) }),
    });
    await harness.client.login();
    const pending = takePending(harness.storage, 1_000_000)!;
    savePending(harness.storage, pending);
    await expect(
      harness.client.handleCallback(`${CALLBACK_URL}?code=c&state=${pending.state}`),
    ).rejects.toMatchObject({ code: "nonce_mismatch" });
    expect(harness.client.hasSession).toBe(false);
  });

  it("rejects an ID token with the wrong audience", async () => {
    const harness = createHarness({
      token: (_body, nonce) => ({
        access_token: "access-1",
        id_token: idToken({ nonce, aud: "other" }),
      }),
    });
    await harness.client.login();
    const pending = takePending(harness.storage, 1_000_000)!;
    savePending(harness.storage, pending);
    await expect(
      harness.client.handleCallback(`${CALLBACK_URL}?code=c&state=${pending.state}`),
    ).rejects.toMatchObject({ code: "id_token_audience_mismatch" });
  });

  it("rejects a token endpoint error", async () => {
    const harness = createHarness({
      token: () => ({ error: "invalid_grant", status: 400 }),
    });
    await harness.client.login();
    const pending = takePending(harness.storage, 1_000_000)!;
    savePending(harness.storage, pending);
    await expect(
      harness.client.handleCallback(`${CALLBACK_URL}?code=c&state=${pending.state}`),
    ).rejects.toMatchObject({ code: "invalid_grant" });
  });

  it("rejects a token response without an access token", async () => {
    const harness = createHarness({ token: () => ({ refresh_token: "refresh-1" }) });
    await harness.client.login();
    const pending = takePending(harness.storage, 1_000_000)!;
    savePending(harness.storage, pending);
    await expect(
      harness.client.handleCallback(`${CALLBACK_URL}?code=c&state=${pending.state}`),
    ).rejects.toBeInstanceOf(OidcError);
  });
});

describe("OidcClient.getAccessToken", () => {
  it("returns null before login", async () => {
    const harness = createHarness();
    await expect(harness.client.getAccessToken()).resolves.toBeNull();
  });

  it("refreshes an expiring token once for concurrent callers", async () => {
    const harness = createHarness({
      token: (body, nonce) => {
        if (body.get("grant_type") === "refresh_token") {
          return { access_token: "access-2", refresh_token: "refresh-2", expires_in: 300 };
        }
        return {
          access_token: "access-1",
          refresh_token: "refresh-1",
          id_token: idToken({ nonce }),
          expires_in: 300,
        };
      },
    });
    await completeLogin(harness);

    harness.advance(300_000 - 30_000);
    const [first, second] = await Promise.all([
      harness.client.getAccessToken(),
      harness.client.getAccessToken(),
    ]);

    expect(first).toBe("access-2");
    expect(second).toBe("access-2");
    expect(harness.refreshCalls()).toHaveLength(1);
    expect(harness.refreshCalls()[0]?.get("refresh_token")).toBe("refresh-1");
  });

  it("keeps the previous id_token when the refresh response omits it", async () => {
    const harness = createHarness({
      token: (body, nonce) => {
        if (body.get("grant_type") === "refresh_token") {
          return { access_token: "access-2", refresh_token: "refresh-2", expires_in: 300 };
        }
        return {
          access_token: "access-1",
          refresh_token: "refresh-1",
          id_token: idToken({ nonce }),
          expires_in: 120,
        };
      },
    });
    await completeLogin(harness);

    harness.advance(70_000);
    await expect(harness.client.getAccessToken()).resolves.toBe("access-2");

    harness.assigned.length = 0;
    await harness.client.logout();
    const url = new URL(harness.assigned[0]!);
    expect(url.searchParams.get("id_token_hint")).not.toBeNull();
  });

  it("drops the session when the refresh fails", async () => {
    const harness = createHarness({
      token: (body, nonce) => {
        if (body.get("grant_type") === "refresh_token") {
          return { error: "invalid_grant", status: 400 };
        }
        return {
          access_token: "access-1",
          refresh_token: "refresh-1",
          id_token: idToken({ nonce }),
          expires_in: 10,
        };
      },
    });
    await completeLogin(harness);

    harness.advance(11_000);
    await expect(harness.client.getAccessToken()).resolves.toBeNull();
    expect(harness.client.hasSession).toBe(false);
  });

  it("discards a refresh that completes after the session was reset", async () => {
    let release!: () => void;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    const harness = createHarness({
      token: async (body, nonce) => {
        if (body.get("grant_type") === "refresh_token") {
          await gate;
          return { access_token: "access-2", refresh_token: "refresh-2", expires_in: 300 };
        }
        return {
          access_token: "access-1",
          refresh_token: "refresh-1",
          id_token: idToken({ nonce }),
          expires_in: 10,
        };
      },
    });
    await completeLogin(harness);

    harness.advance(11_000);
    const refreshing = harness.client.getAccessToken();
    await Promise.resolve();
    harness.client.resetSession();
    release();

    await expect(refreshing).resolves.toBeNull();
    expect(harness.client.hasSession).toBe(false);
  });

  it("drops an expired token that has no refresh token", async () => {
    const harness = createHarness({
      token: (_body, nonce) => ({
        access_token: "access-1",
        id_token: idToken({ nonce }),
        expires_in: 10,
      }),
    });
    await completeLogin(harness);

    harness.advance(11_000);
    await expect(harness.client.getAccessToken()).resolves.toBeNull();
    expect(harness.client.hasSession).toBe(false);
    expect(harness.refreshCalls()).toHaveLength(0);
  });
});

describe("OidcClient.logout", () => {
  it("clears the session and redirects to end_session_endpoint when present", async () => {
    const harness = createHarness({
      token: (_body, nonce) => ({
        access_token: "access-1",
        id_token: idToken({ nonce }),
        expires_in: 300,
      }),
    });
    await completeLogin(harness);
    harness.assigned.length = 0;

    await harness.client.logout();

    expect(harness.client.hasSession).toBe(false);
    const url = new URL(harness.assigned[0]!);
    expect(url.origin + url.pathname).toBe(END_SESSION_ENDPOINT);
    expect(url.searchParams.get("client_id")).toBe(CLIENT_ID);
    expect(url.searchParams.get("post_logout_redirect_uri")).toBe("https://app.example/");
    expect(url.searchParams.get("id_token_hint")).not.toBeNull();
    expect(harness.storage.getItem("zoo.oidc.loggedOut")).toBe("1");
  });

  it("falls back to the post-logout redirect when the IdP has no logout endpoint", async () => {
    const harness = createHarness({
      discovery: discoveryBody({ end_session_endpoint: undefined }),
    });
    await harness.client.logout();
    expect(harness.assigned[0]).toBe("https://app.example/");
  });
});

describe("OidcClient.isCallbackUrl", () => {
  it("matches only the callback path with code or error", () => {
    const harness = createHarness();
    expect(harness.client.isCallbackUrl(`${CALLBACK_URL}?code=c`)).toBe(true);
    expect(harness.client.isCallbackUrl(`${CALLBACK_URL}?error=x`)).toBe(true);
    expect(harness.client.isCallbackUrl(CALLBACK_URL)).toBe(false);
    expect(harness.client.isCallbackUrl("https://app.example/channels/abc")).toBe(false);
    expect(harness.client.isCallbackUrl("https://other.example/auth/callback?code=c")).toBe(false);
  });
});
