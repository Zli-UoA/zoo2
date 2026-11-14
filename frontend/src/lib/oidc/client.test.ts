import { describe, expect, it, vi } from "vite-plus/test";

import {
  DEFAULT_CALLBACK_PATH,
  OidcClient,
  OidcError,
  sanitizeReturnTo,
  type OidcEnvironment,
  type OidcFetch,
} from "./client";
import { isLoggedOut, savePending, takePending, type AuthStorage } from "./pending";
import { base64UrlEncode, createChallenge } from "./pkce";

const ISSUER = "https://id.example/oauth2/openid/zoo";
const CLIENT_ID = "zoo";
const AUTHZ_ENDPOINT = "https://id.example/ui/oauth2";
const TOKEN_ENDPOINT = "https://id.example/oauth2/token";
const END_SESSION_ENDPOINT = "https://id.example/ui/logout";
const CALLBACK_URL = "https://app.example/auth/callback";
const RESOURCE = "https://zoo2.example";

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
  reauthorizeCalls: string[];
  fetchMock: ReturnType<typeof vi.fn<OidcFetch>>;
  tokenCalls: URLSearchParams[];
  advance(ms: number): void;
  refreshCalls(): URLSearchParams[];
  /** token ハンドラが参照する nonce を、login 後に設定する。 */
  setNonce(nonce: string): void;
  setHref(href: string): void;
  setOnline(online: boolean): void;
}

interface HarnessOptions {
  storage?: AuthStorage;
  now?: number;
  online?: boolean;
  discovery?: Record<string, unknown>;
  discoveryWait?: Promise<void>;
  /** token エンドポイントの応答。`URLSearchParams` と現在の nonce を受け取る。 */
  token?: (body: URLSearchParams, nonce: string) => unknown;
  resource?: string;
  href?: string;
}

function createHarness(options: HarnessOptions = {}): Harness {
  const storage = options.storage ?? memoryStorage();
  const assigned: string[] = [];
  const reauthorizeCalls: string[] = [];
  const tokenCalls: URLSearchParams[] = [];
  let clock = options.now ?? 1_000_000;
  let online = options.online ?? true;
  let lastNonce = "nonce";
  let href = options.href ?? "https://app.example/";

  const fetchMock = vi.fn<OidcFetch>(async (input, init) => {
    if (input.endsWith("/.well-known/openid-configuration")) {
      await options.discoveryWait;
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
    get href() {
      return href;
    },
    assign: (url) => void assigned.push(url),
    replaceUrl: () => {},
    storage,
    fetch: fetchMock,
    now: () => clock,
    isOnline: () => online,
  };
  const client = new OidcClient({
    issuer: ISSUER,
    clientId: CLIENT_ID,
    environment: env,
    resource: options.resource,
  });
  client.setReauthorizeHandler((returnTo) => {
    reauthorizeCalls.push(returnTo);
  });

  return {
    client,
    storage,
    assigned,
    reauthorizeCalls,
    fetchMock,
    tokenCalls,
    advance: (ms) => {
      clock += ms;
    },
    refreshCalls: () => tokenCalls.filter((body) => body.get("grant_type") === "refresh_token"),
    setNonce: (value) => {
      lastNonce = value;
    },
    setHref: (value) => {
      href = value;
    },
    setOnline: (value) => {
      online = value;
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

describe("OidcClient session restoration", () => {
  it.each([false, true])(
    "defers offline reauthorization and refresh until reconnection (refresh: %s)",
    async (withRefresh) => {
      const original = createHarness({
        token: (body, nonce) =>
          body.get("grant_type") === "refresh_token"
            ? { access_token: "access-2", refresh_token: "refresh-2", expires_in: 300 }
            : {
                access_token: "access-1",
                id_token: idToken({ nonce }),
                expires_in: 90,
                ...(withRefresh ? { refresh_token: "refresh-1" } : {}),
              },
      });
      await completeLogin(original);
      const reloaded = withRefresh
        ? original
        : createHarness({
            storage: original.storage,
            now: 1_040_000,
            online: false,
          });
      if (withRefresh) {
        reloaded.advance(40_000);
        reloaded.setOnline(false);
        reloaded.fetchMock.mockClear();
      }
      await expect(reloaded.client.getAccessToken()).resolves.toBe("access-1");
      reloaded.advance(51_000);
      await expect(reloaded.client.getAccessToken()).resolves.toBeNull();
      expect(reloaded.client.hasSession).toBe(true);
      expect(reloaded.fetchMock).not.toHaveBeenCalled();
      expect(reloaded.reauthorizeCalls).toEqual([]);
      reloaded.setOnline(true);
      await expect(reloaded.client.getAccessToken()).resolves.toBe(withRefresh ? "access-2" : null);
      expect(reloaded.refreshCalls()).toHaveLength(withRefresh ? 1 : 0);
      expect(reloaded.reauthorizeCalls).toEqual(withRefresh ? [] : ["/"]);
    },
  );

  it("restores the session after reload without contacting the IdP or redirecting", async () => {
    const original = createHarness();
    await completeLogin(original);
    const reloaded = createHarness({ storage: original.storage });
    expect(reloaded.client.hasSession).toBe(true);
    await expect(reloaded.client.getAccessToken()).resolves.toBe("access-1");
    expect(reloaded.fetchMock).not.toHaveBeenCalled();
    expect(reloaded.assigned).toEqual([]);
    expect(reloaded.reauthorizeCalls).toEqual([]);
  });

  it("does not copy a rotating refresh token into duplicated tabs", async () => {
    const original = createHarness({
      token: (body, nonce) =>
        body.get("grant_type") === "refresh_token"
          ? { access_token: "access-2", refresh_token: "refresh-2", expires_in: 300 }
          : {
              access_token: "access-1",
              refresh_token: "refresh-1",
              id_token: idToken({ nonce }),
              expires_in: 90,
            },
    });
    await completeLogin(original);
    const copiedStorage = memoryStorage();
    const saved = original.storage.getItem("zoo.oidc.session")!;
    expect(JSON.parse(saved).tokens.refreshToken).toBeNull();
    copiedStorage.setItem("zoo.oidc.session", saved);
    const duplicated = createHarness({ storage: copiedStorage, now: 1_040_000 });
    original.advance(40_000);
    await expect(original.client.getAccessToken()).resolves.toBe("access-2");
    await expect(duplicated.client.getAccessToken()).resolves.toBe("access-1");
    expect(original.refreshCalls()).toHaveLength(1);
    expect(duplicated.refreshCalls()).toHaveLength(0);
    expect(duplicated.reauthorizeCalls).toEqual(["/"]);
    const reloaded = createHarness({ storage: original.storage, now: 1_040_000 });
    await expect(reloaded.client.getAccessToken()).resolves.toBe("access-2");
    expect(reloaded.fetchMock).not.toHaveBeenCalled();
  });

  it("does not restore tokens for different resource settings", async () => {
    const original = createHarness({ resource: RESOURCE });
    await completeLogin(original);
    expect(createHarness({ storage: original.storage }).client.hasSession).toBe(false);
  });

  it("does not restore expired tokens without a refresh token", async () => {
    const original = createHarness();
    await completeLogin(original);
    expect(createHarness({ storage: original.storage, now: 1_400_000 }).client.hasSession).toBe(
      false,
    );
  });

  it.each(["reset", "logout", "logged-out marker"])(
    "does not restore a session after %s",
    async (action) => {
      const original = createHarness();
      await completeLogin(original);
      if (action === "reset") original.client.resetSession();
      else if (action === "logout") await original.client.logout();
      else original.storage.setItem("zoo.oidc.loggedOut", "1");
      expect(createHarness({ storage: original.storage }).client.hasSession).toBe(false);
    },
  );

  it("clears the saved session when an in-memory refresh token is rejected", async () => {
    const original = createHarness({
      token: (body, nonce) =>
        body.get("grant_type") === "refresh_token"
          ? { error: "invalid_grant", status: 400 }
          : {
              access_token: "access-1",
              refresh_token: "refresh-1",
              id_token: idToken({ nonce }),
              expires_in: 10,
            },
    });
    await completeLogin(original);
    original.advance(20_000);
    await expect(original.client.getAccessToken()).resolves.toBeNull();
    expect(createHarness({ storage: original.storage }).client.hasSession).toBe(false);
  });
});

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
    expect(url.searchParams.get("resource")).toBeNull();
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

  it("rejects a pre-logout callback after a one-time pending-removal failure but allows a new login", async () => {
    const previous = createHarness();
    await previous.client.login("/channels/old");
    const oldPending = takePending(previous.storage, 1_000_001);
    if (!oldPending) throw new Error("old pending authorization was not stored");
    savePending(previous.storage, oldPending);
    const originalRemove = previous.storage.removeItem.bind(previous.storage);
    let denyPendingRemoval = true;
    const remove = vi.spyOn(previous.storage, "removeItem").mockImplementation((key) => {
      if (key === "zoo.oidc.pending" && denyPendingRemoval) {
        denyPendingRemoval = false;
        throw new Error("transient storage failure");
      }
      originalRemove(key);
    });

    try {
      await previous.client.logout();
      expect(isLoggedOut(previous.storage)).toBe(true);
      expect(previous.storage.getItem("zoo.oidc.pending")).not.toBeNull();

      const resumed = createHarness({ storage: previous.storage });
      const oldCallback = `${CALLBACK_URL}?code=old-code&state=${oldPending.state}`;
      await expect(resumed.client.handleCallback(oldCallback)).rejects.toMatchObject({
        code: "session_reset",
      });
      expect(resumed.tokenCalls).toHaveLength(0);
      expect(isLoggedOut(previous.storage)).toBe(true);

      await resumed.client.login("/channels/new");
      expect(isLoggedOut(previous.storage)).toBe(false);
      const newPending = takePending(previous.storage, 1_000_001);
      if (!newPending) throw new Error("new pending authorization was not stored");
      savePending(previous.storage, newPending);
      resumed.setNonce(newPending.nonce);
      const newCallback = `${CALLBACK_URL}?code=new-code&state=${newPending.state}`;
      await expect(resumed.client.handleCallback(newCallback)).resolves.toEqual({
        returnTo: "/channels/new",
      });
      expect(resumed.client.hasSession).toBe(true);
    } finally {
      remove.mockRestore();
    }
  });

  it("does not persist or redirect after logout while discovery is pending", async () => {
    let releaseDiscovery!: () => void;
    let discoveryStarted!: () => void;
    const discoveryWait = new Promise<void>((resolve) => {
      releaseDiscovery = resolve;
    });
    const started = new Promise<void>((resolve) => {
      discoveryStarted = resolve;
    });
    const harness = createHarness({ discoveryWait });
    harness.fetchMock.mockImplementationOnce(async (input) => {
      if (input.endsWith("/.well-known/openid-configuration")) discoveryStarted();
      await discoveryWait;
      return jsonResponse(discoveryBody());
    });

    const oldLogin = harness.client.login("/channels/private");
    await started;
    const logout = harness.client.logout();
    releaseDiscovery();

    await logout;
    await expect(oldLogin).rejects.toMatchObject({ code: "session_reset" });
    expect(takePending(harness.storage, 1_000_001)).toBeNull();
    expect(harness.assigned).toHaveLength(1);
    expect(harness.assigned[0]).toContain("/ui/logout");
  });

  it("lets a newer login supersede a logout waiting for discovery", async () => {
    let releaseDiscovery!: () => void;
    let discoveryStarted!: () => void;
    const discoveryWait = new Promise<void>((resolve) => {
      releaseDiscovery = resolve;
    });
    const started = new Promise<void>((resolve) => {
      discoveryStarted = resolve;
    });
    const harness = createHarness({ discoveryWait });
    harness.fetchMock.mockImplementationOnce(async (input) => {
      if (input.endsWith("/.well-known/openid-configuration")) discoveryStarted();
      await discoveryWait;
      return jsonResponse(discoveryBody());
    });

    const logout = harness.client.logout();
    await started;
    const login = harness.client.login("/channels/new");
    releaseDiscovery();

    await expect(logout).resolves.toBeUndefined();
    const authorizeUrl = await login;
    expect(harness.assigned).toEqual([authorizeUrl]);
    expect(new URL(authorizeUrl).pathname).toBe("/ui/oauth2");
    expect(isLoggedOut(harness.storage)).toBe(false);
  });

  it("does not persist or redirect after logout while PKCE is pending", async () => {
    const harness = createHarness();
    await harness.client.discover();
    let releaseDigest!: (digest: ArrayBuffer) => void;
    let digestStarted!: () => void;
    const started = new Promise<void>((resolve) => {
      digestStarted = resolve;
    });
    const digestWait = new Promise<ArrayBuffer>((resolve) => {
      releaseDigest = resolve;
    });
    const digest = vi.spyOn(globalThis.crypto.subtle, "digest").mockImplementation(async () => {
      digestStarted();
      return await digestWait;
    });

    const oldLogin = harness.client.login("/channels/private");
    await started;
    await harness.client.logout();
    releaseDigest(new Uint8Array(32).buffer);
    try {
      await expect(oldLogin).rejects.toMatchObject({ code: "session_reset" });
      expect(takePending(harness.storage, 1_000_001)).toBeNull();
      expect(harness.assigned).toHaveLength(1);
      expect(harness.assigned[0]).toContain("/ui/logout");
    } finally {
      digest.mockRestore();
    }
  });

  it("includes the resource in authorize and token requests when configured", async () => {
    const harness = createHarness({ resource: RESOURCE });
    const href = await harness.client.login();

    expect(new URL(href).searchParams.get("resource")).toBe(RESOURCE);

    const pending = takePending(harness.storage, 1_000_000)!;
    savePending(harness.storage, pending);
    harness.setNonce(pending.nonce);
    await harness.client.handleCallback(`${CALLBACK_URL}?code=code-1&state=${pending.state}`);

    expect(harness.tokenCalls[0]?.get("resource")).toBe(RESOURCE);
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
    expect(body?.get("resource")).toBeNull();
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

  it("keeps the new pending request when an old authorization error arrives", async () => {
    const harness = createHarness();
    await harness.client.login("/channels/new");
    const pending = takePending(harness.storage, 1_000_001)!;
    savePending(harness.storage, pending);
    harness.setNonce(pending.nonce);
    await expect(
      harness.client.handleCallback(`${CALLBACK_URL}?error=access_denied&state=old`),
    ).rejects.toMatchObject({ code: "state_mismatch" });
    await expect(
      harness.client.handleCallback(`${CALLBACK_URL}?code=new&state=${pending.state}`),
    ).resolves.toEqual({ returnTo: "/channels/new" });
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

  it("preserves a new pending request when an old callback arrives first", async () => {
    const harness = createHarness();
    await harness.client.login("/channels/old");
    const oldPending = takePending(harness.storage, 1_000_001);
    if (!oldPending) throw new Error("old pending authorization was not stored");
    savePending(harness.storage, oldPending);
    await harness.client.logout();

    await harness.client.login("/channels/new");
    const newPending = takePending(harness.storage, 1_000_001);
    if (!newPending) throw new Error("new pending authorization was not stored");
    savePending(harness.storage, newPending);
    harness.setNonce(newPending.nonce);

    await expect(
      harness.client.handleCallback(`${CALLBACK_URL}?code=old&state=${oldPending.state}`),
    ).rejects.toMatchObject({ code: "state_mismatch" });
    const preservedPending = takePending(harness.storage, 1_000_001);
    expect(preservedPending?.state).toBe(newPending.state);
    if (preservedPending) savePending(harness.storage, preservedPending);

    await expect(
      harness.client.handleCallback(`${CALLBACK_URL}?code=new&state=${newPending.state}`),
    ).resolves.toEqual({ returnTo: "/channels/new" });
    expect(harness.tokenCalls.map((body) => body.get("code"))).toEqual(["new"]);
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

  it("does not reuse a refreshing token while login is preparing", async () => {
    let releaseRefresh!: () => void;
    let releaseDigest!: (digest: ArrayBuffer) => void;
    let digestStarted!: () => void;
    const refreshGate = new Promise<void>((resolve) => {
      releaseRefresh = resolve;
    });
    const digestGate = new Promise<ArrayBuffer>((resolve) => {
      releaseDigest = resolve;
    });
    const digestReady = new Promise<void>((resolve) => {
      digestStarted = resolve;
    });
    const harness = createHarness({
      token: async (body, nonce) => {
        if (body.get("grant_type") === "refresh_token") {
          await refreshGate;
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
    await vi.waitFor(() => expect(harness.refreshCalls()).toHaveLength(1));
    const digest = vi.spyOn(globalThis.crypto.subtle, "digest").mockImplementation(async () => {
      digestStarted();
      return await digestGate;
    });
    try {
      const login = harness.client.login();
      await digestReady;
      const access = harness.client.getAccessToken();
      await Promise.resolve();
      expect(harness.refreshCalls()).toHaveLength(1);
      releaseRefresh();
      releaseDigest(new Uint8Array(32).buffer);
      await expect(refreshing).resolves.toBeNull();
      await expect(access).resolves.toBeNull();
      await expect(login).resolves.toContain("/ui/oauth2");
      await expect(harness.client.getAccessToken()).resolves.toBeNull();
      expect(harness.refreshCalls()).toHaveLength(1);
    } finally {
      releaseRefresh();
      releaseDigest(new Uint8Array(32).buffer);
      digest.mockRestore();
    }
  });

  it("returns an unexpired access token without refreshing during login preparation", async () => {
    const harness = createHarness({
      token: (_body, nonce) => ({
        access_token: "access-1",
        refresh_token: "refresh-1",
        id_token: idToken({ nonce }),
        expires_in: 30,
      }),
    });
    await completeLogin(harness);
    let started!: () => void;
    let release!: (digest: ArrayBuffer) => void;
    const ready = new Promise<void>((resolve) => {
      started = resolve;
    });
    const waiting = new Promise<ArrayBuffer>((resolve) => {
      release = resolve;
    });
    const digest = vi.spyOn(globalThis.crypto.subtle, "digest").mockImplementation(async () => {
      started();
      return await waiting;
    });
    try {
      const login = harness.client.login();
      await ready;
      await expect(harness.client.getAccessToken()).resolves.toBe("access-1");
      expect(harness.refreshCalls()).toHaveLength(0);
      harness.advance(30_000);
      await expect(harness.client.getAccessToken()).resolves.toBeNull();
      expect(harness.refreshCalls()).toHaveLength(0);
      release(new Uint8Array(32).buffer);
      await login;
    } finally {
      release(new Uint8Array(32).buffer);
      digest.mockRestore();
    }
  });

  it("does not let an old refresh failure clear a newly authenticated session", async () => {
    let release!: () => void;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    let authorizationCodeExchanges = 0;
    const harness = createHarness({
      token: async (body, nonce) => {
        if (body.get("grant_type") === "refresh_token") {
          await gate;
          return {
            access_token: "stale-refresh",
            refresh_token: "stale-refresh-2",
            expires_in: 300,
          };
        }
        authorizationCodeExchanges += 1;
        return {
          access_token: `access-${authorizationCodeExchanges}`,
          refresh_token: `refresh-${authorizationCodeExchanges}`,
          id_token: idToken({ nonce }),
          expires_in: authorizationCodeExchanges === 1 ? 10 : 300,
        };
      },
    });
    await completeLogin(harness);

    harness.advance(11_000);
    const oldRefresh = harness.client.getAccessToken();
    await Promise.resolve();
    harness.client.resetSession();
    await completeLogin(harness);
    release();

    await expect(oldRefresh).resolves.toBeNull();
    expect(harness.client.hasSession).toBe(true);
    await expect(harness.client.getAccessToken()).resolves.toBe("access-2");
  });

  it("discards an authorization response that arrives after logout", async () => {
    let release!: () => void;
    let started!: () => void;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    const requestStarted = new Promise<void>((resolve) => {
      started = resolve;
    });
    const harness = createHarness({
      token: async (body, nonce) => {
        if (body.get("grant_type") === "authorization_code") {
          started();
          await gate;
          return {
            access_token: "old-session-token",
            refresh_token: "old-session-refresh",
            id_token: idToken({ nonce }),
            expires_in: 300,
          };
        }
        return {};
      },
    });
    await harness.client.login();
    const pending = takePending(harness.storage, 1_000_001);
    if (!pending) throw new Error("pending was not stored");
    savePending(harness.storage, pending);
    harness.setNonce(pending.nonce);
    const callback = harness.client.handleCallback(
      `${CALLBACK_URL}?code=old-code&state=${pending.state}`,
    );
    await requestStarted;

    await harness.client.logout();
    release();

    await expect(callback).rejects.toMatchObject({ code: "session_reset" });
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

describe("OidcClient proactive reauthorization", () => {
  it("does not request reauthorization while the token is comfortably valid", async () => {
    const harness = createHarness();
    await completeLogin(harness);

    harness.advance(239_000);
    await expect(harness.client.getAccessToken()).resolves.toBe("access-1");
    expect(harness.reauthorizeCalls).toHaveLength(0);
  });

  it("starts reauthorization inside the skew window and keeps the usable token", async () => {
    const harness = createHarness();
    await completeLogin(harness);

    harness.advance(240_000);
    await expect(harness.client.getAccessToken()).resolves.toBe("access-1");
    expect(harness.reauthorizeCalls).toHaveLength(1);
    expect(harness.client.hasSession).toBe(true);
  });

  it("starts reauthorization at expiry and does not return the expired token", async () => {
    const harness = createHarness();
    await completeLogin(harness);

    harness.advance(300_000);
    await expect(harness.client.getAccessToken()).resolves.toBeNull();
    expect(harness.reauthorizeCalls).toHaveLength(1);
    expect(harness.client.hasSession).toBe(false);
  });

  it("requests reauthorization after a refresh transport failure", async () => {
    const harness = createHarness({
      token: async (body, nonce) => {
        if (body.get("grant_type") === "refresh_token") {
          throw new TypeError("network unavailable");
        }
        return {
          access_token: "access-1",
          refresh_token: "refresh-1",
          id_token: idToken({ nonce }),
          expires_in: 10,
        };
      },
    });
    const reauthorize = vi.fn();
    await completeLogin(harness);
    harness.client.setReauthorizeHandler(reauthorize);
    harness.advance(11_000);

    await expect(harness.client.getAccessToken()).resolves.toBeNull();

    expect(harness.client.hasSession).toBe(false);
    expect(reauthorize).toHaveBeenCalledTimes(1);
  });

  it("starts reauthorization after expiry", async () => {
    const harness = createHarness();
    await completeLogin(harness);

    harness.advance(300_001);
    await expect(harness.client.getAccessToken()).resolves.toBeNull();
    expect(harness.reauthorizeCalls).toHaveLength(1);
  });

  it("coalesces concurrent calls into one reauthorization", async () => {
    const harness = createHarness();
    await completeLogin(harness);

    harness.advance(250_000);
    const tokens = await Promise.all([
      harness.client.getAccessToken(),
      harness.client.getAccessToken(),
      harness.client.getAccessToken(),
    ]);

    expect(tokens).toEqual(["access-1", "access-1", "access-1"]);
    expect(harness.reauthorizeCalls).toHaveLength(1);
  });

  it("passes the current path as returnTo", async () => {
    const harness = createHarness({ href: "https://app.example/channels/abc?tab=info#top" });
    await completeLogin(harness);

    harness.advance(290_000);
    await harness.client.getAccessToken();

    expect(harness.reauthorizeCalls).toEqual(["/channels/abc?tab=info#top"]);
  });

  it("keeps returnTo across a proactive reauthorization", async () => {
    const harness = createHarness({ href: "https://app.example/channels/abc?tab=info#top" });
    await completeLogin(harness);
    harness.assigned.length = 0;
    harness.client.setReauthorizeHandler((returnTo) => {
      void harness.client.login(returnTo);
    });

    harness.advance(290_000);
    await expect(harness.client.getAccessToken()).resolves.toBe("access-1");
    await vi.waitFor(() => expect(harness.assigned).toHaveLength(1));

    const pending = takePending(harness.storage, 1_290_000);
    expect(pending?.returnTo).toBe("/channels/abc?tab=info#top");

    savePending(harness.storage, pending!);
    harness.setNonce(pending!.nonce);
    const result = await harness.client.handleCallback(
      `${CALLBACK_URL}?code=code-2&state=${pending!.state}`,
    );
    expect(result.returnTo).toBe("/channels/abc?tab=info#top");
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
    expect(url.searchParams.get("state")).toBeTruthy();
    expect(url.searchParams.get("id_token_hint")).not.toBeNull();
    expect(harness.storage.getItem("zoo.oidc.loggedOut")).toBe("1");
  });

  it("blocks logout until the stored session is deleted even if the logout marker succeeds", async () => {
    const baseStorage = memoryStorage();
    let denySessionDeletion = false;
    const storage: AuthStorage = {
      getItem: (key) => baseStorage.getItem(key),
      setItem: (key, value) => baseStorage.setItem(key, value),
      removeItem: (key) => {
        if (denySessionDeletion && key === "zoo.oidc.session") throw new Error("storage denied");
        baseStorage.removeItem(key);
      },
    };
    const harness = createHarness({ storage });
    await completeLogin(harness);
    harness.assigned.length = 0;
    expect(storage.getItem("zoo.oidc.session")).not.toBeNull();
    denySessionDeletion = true;
    await harness.client.logout();
    expect(harness.client.hasSession).toBe(false);
    expect(storage.getItem("zoo.oidc.loggedOut")).toBe("1");
    expect(harness.assigned).toHaveLength(0);
    const reloaded = createHarness({ storage });
    await expect(reloaded.client.login()).rejects.toThrow();
    expect(storage.getItem("zoo.oidc.loggedOut")).toBe("1");
    expect(reloaded.assigned).toHaveLength(0);
    denySessionDeletion = false;
    await harness.client.logout();
    expect(storage.getItem("zoo.oidc.session")).toBeNull();
    expect(harness.assigned).toHaveLength(1);
  });

  it("keeps only the logout hint through storage failure and consumes it once after retry", async () => {
    const baseStorage = memoryStorage();
    let storageDenied = false;
    const storage: AuthStorage = {
      getItem: (key) => baseStorage.getItem(key),
      setItem: (key, value) => {
        if (storageDenied) throw new Error("storage denied");
        baseStorage.setItem(key, value);
      },
      removeItem: (key) => {
        if (storageDenied) throw new Error("storage denied");
        baseStorage.removeItem(key);
      },
    };
    let issuedIdToken = "";
    const harness = createHarness({
      storage,
      token: (_body, nonce) => {
        issuedIdToken = idToken({ nonce });
        return { access_token: "access-1", id_token: issuedIdToken, expires_in: 300 };
      },
    });
    await completeLogin(harness);
    harness.assigned.length = 0;

    storageDenied = true;
    await harness.client.logout();
    expect(harness.client.hasSession).toBe(false);
    await expect(harness.client.getAccessToken()).resolves.toBeNull();
    expect(harness.assigned).toHaveLength(0);

    storageDenied = false;
    await harness.client.logout();
    const retryLogoutUrl = new URL(harness.assigned[0]!);
    expect(retryLogoutUrl.searchParams.get("id_token_hint")).toBe(issuedIdToken);

    await harness.client.logout();
    const laterLogoutUrl = new URL(harness.assigned[1]!);
    expect(laterLogoutUrl.searchParams.get("id_token_hint")).toBeNull();
  });

  it("drops a blocked logout hint when a new user login replaces the session", async () => {
    const baseStorage = memoryStorage();
    let storageDenied = false;
    const storage: AuthStorage = {
      getItem: (key) => baseStorage.getItem(key),
      setItem: (key, value) => {
        if (storageDenied) throw new Error("storage denied");
        baseStorage.setItem(key, value);
      },
      removeItem: (key) => {
        if (storageDenied) throw new Error("storage denied");
        baseStorage.removeItem(key);
      },
    };
    const issuedIdTokens: string[] = [];
    const harness = createHarness({
      storage,
      token: (_body, nonce) => {
        const issuedIdToken = idToken({ nonce });
        issuedIdTokens.push(issuedIdToken);
        return { access_token: "access-1", id_token: issuedIdToken, expires_in: 300 };
      },
    });
    await completeLogin(harness);
    storageDenied = true;
    await harness.client.logout();
    storageDenied = false;

    await completeLogin(harness);
    expect(issuedIdTokens).toHaveLength(2);
    await harness.client.logout();

    const logoutUrl = new URL(harness.assigned.at(-1)!);
    expect(logoutUrl.searchParams.get("id_token_hint")).toBe(issuedIdTokens[1]);
    expect(logoutUrl.searchParams.get("id_token_hint")).not.toBe(issuedIdTokens[0]);
  });

  it("discards a blocked logout hint when the session is reset", async () => {
    const baseStorage = memoryStorage();
    let storageDenied = false;
    const storage: AuthStorage = {
      getItem: (key) => baseStorage.getItem(key),
      setItem: (key, value) => {
        if (storageDenied) throw new Error("storage denied");
        baseStorage.setItem(key, value);
      },
      removeItem: (key) => {
        if (storageDenied) throw new Error("storage denied");
        baseStorage.removeItem(key);
      },
    };
    const harness = createHarness({
      storage,
      token: (_body, nonce) => ({
        access_token: "access-1",
        id_token: idToken({ nonce }),
        expires_in: 300,
      }),
    });
    await completeLogin(harness);
    storageDenied = true;
    await harness.client.logout();
    storageDenied = false;

    harness.client.resetSession();
    await harness.client.logout();

    expect(new URL(harness.assigned.at(-1)!).searchParams.get("id_token_hint")).toBeNull();
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
