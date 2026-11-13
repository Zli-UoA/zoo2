// @vitest-environment happy-dom
import { act, createElement, type ButtonHTMLAttributes, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vite-plus/test";

import { AuthGate } from "./AuthGate";
import { createApiClient, type FetchLike } from "../../lib/api/client";
import { subscribeAuthSessionEnded } from "../../lib/auth";
import { createShapeFetch } from "../../lib/electric/headers";
import { OidcClient, type OidcFetch } from "../../lib/oidc/client";
import { isLoggedOut, markRedirectStarted, savePending, takePending } from "../../lib/oidc/pending";
import { base64UrlEncode } from "../../lib/oidc/pkce";

const ORIGIN = "https://app.example";
const ISSUER = "https://id.example";
const CALLBACK = `${ORIGIN}/auth/callback`;

const authMocks = vi.hoisted(() => ({
  client: null as unknown,
  clientFactory: null as (() => unknown) | null,
  router: {
    history: { replace: vi.fn() },
    load: vi.fn(async () => {}),
  },
  translate: (key: string) => key,
}));

vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: authMocks.translate }) }));
vi.mock("@tanstack/react-router", () => ({ useRouter: () => authMocks.router }));
vi.mock("../../lib/config", () => ({
  resolveConfig: () => ({
    apiBaseUrl: "https://api.example",
    devUserId: null,
    oidc: { issuer: "https://id.example", clientId: "zoo" },
  }),
  joinUrl: (baseUrl: string, path: string) =>
    `${baseUrl.replace(/\/$/, "")}/${path.replace(/^\//, "")}`,
}));
vi.mock("../../lib/oidc/session", () => ({
  getOidcClient: () => authMocks.clientFactory?.() ?? authMocks.client,
}));
vi.mock("../i18n/LanguageToggle", () => ({ LanguageToggle: () => null }));
vi.mock("../layout/ShellSkeleton", () => ({
  ShellSkeleton: () => createElement("div", null, "auth-checking"),
}));
vi.mock("../ui/button", () => ({
  Button: ({
    children,
    ...props
  }: { children?: ReactNode } & ButtonHTMLAttributes<HTMLButtonElement>) =>
    createElement("button", props, children),
}));

function jsonResponse(body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "Content-Type": "application/json" },
  });
}

function makeIdToken(nonce: string): string {
  const encode = (value: unknown) =>
    base64UrlEncode(new TextEncoder().encode(JSON.stringify(value)));
  return `${encode({ alg: "ES256", typ: "JWT" })}.${encode({
    iss: ISSUER,
    aud: "zoo",
    exp: 9_999_999_999,
    nonce,
  })}.signature`;
}

function createOidcHarness(
  token: (body: URLSearchParams, nonce: string) => Promise<unknown>,
  discovery: {
    wait?: Promise<void>;
    onStart?: () => void;
    respond?: (request: number) => Promise<Response>;
  } = {},
) {
  let href = `${ORIGIN}/channels/private`;
  let now = 1_700_000_000_000;
  let nonce = "initial-nonce";
  let discoveryRequests = 0;
  const assigned: string[] = [];
  const fetch: OidcFetch = async (input, init) => {
    if (input.endsWith("/.well-known/openid-configuration")) {
      discoveryRequests += 1;
      discovery.onStart?.();
      if (discovery.respond) return await discovery.respond(discoveryRequests);
      await discovery.wait;
      return jsonResponse({
        issuer: ISSUER,
        authorization_endpoint: `${ISSUER}/authorize`,
        token_endpoint: `${ISSUER}/token`,
        end_session_endpoint: `${ISSUER}/logout`,
      });
    }
    return jsonResponse(await token(new URLSearchParams(init?.body as string), nonce));
  };
  const client = new OidcClient({
    issuer: ISSUER,
    clientId: "zoo",
    environment: {
      origin: ORIGIN,
      get href() {
        return href;
      },
      assign: (url) => assigned.push(url),
      replaceUrl: vi.fn(),
      storage: window.sessionStorage,
      fetch,
      now: () => now,
    },
  });
  return {
    client,
    assigned,
    get now() {
      return now;
    },
    advance: (ms: number) => {
      now += ms;
    },
    setHref: (value: string) => {
      href = value;
    },
    setNonce: (value: string) => {
      nonce = value;
    },
  };
}

async function completeLogin(harness: ReturnType<typeof createOidcHarness>, code = "initial-code") {
  await harness.client.login();
  const pending = takePending(window.sessionStorage, harness.now + 1);
  if (!pending) throw new Error("pending authorization was not stored");
  savePending(window.sessionStorage, pending);
  harness.setNonce(pending.nonce);
  await harness.client.handleCallback(`${CALLBACK}?code=${code}&state=${pending.state}`);
  return pending;
}

let root: Root;
let container: HTMLDivElement;
let previousOnlineDescriptor: PropertyDescriptor | undefined;

async function flush(action: () => unknown = () => {}) {
  await act(async () => {
    action();
    for (let tick = 0; tick < 12; tick += 1) await Promise.resolve();
  });
}

function setOnline(online: boolean) {
  Object.defineProperty(window.navigator, "onLine", {
    configurable: true,
    value: online,
  });
}

function createSessionStorageGate() {
  const descriptor = Object.getOwnPropertyDescriptor(window, "sessionStorage");
  const original = window.sessionStorage;
  let denied: "writes" | "getItem" | "setItem" | "removeItem" | null = null;
  Object.defineProperty(window, "sessionStorage", {
    configurable: true,
    value: {
      get length() {
        return original.length;
      },
      clear: () => original.clear(),
      getItem: (key: string) => {
        if (denied === "getItem") throw new DOMException("storage denied", "SecurityError");
        return original.getItem(key);
      },
      key: (index: number) => original.key(index),
      removeItem: (key: string) => {
        if (denied === "writes" || denied === "removeItem")
          throw new DOMException("storage denied", "SecurityError");
        original.removeItem(key);
      },
      setItem: (key: string, value: string) => {
        if (denied === "writes" || denied === "setItem")
          throw new DOMException("storage denied", "SecurityError");
        original.setItem(key, value);
      },
    },
  });
  return {
    deny(operation: "writes" | "getItem" | "setItem" | "removeItem" = "writes") {
      denied = operation;
    },
    allow() {
      denied = null;
    },
    restore() {
      if (descriptor) Object.defineProperty(window, "sessionStorage", descriptor);
      else Reflect.deleteProperty(window, "sessionStorage");
    },
  };
}

async function renderGate() {
  await flush(() =>
    root.render(
      createElement(AuthGate, {
        children: createElement("div", null, "private-content"),
      }),
    ),
  );
}

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  sessionStorage.clear();
  vi.clearAllMocks();
  authMocks.client = null;
  authMocks.clientFactory = null;
  previousOnlineDescriptor = Object.getOwnPropertyDescriptor(window.navigator, "onLine");
  setOnline(true);
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(async () => {
  await flush(() => root.unmount());
  container.remove();
  sessionStorage.clear();
  if (previousOnlineDescriptor) {
    Object.defineProperty(window.navigator, "onLine", previousOnlineDescriptor);
  } else {
    Reflect.deleteProperty(window.navigator, "onLine");
  }
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("AuthGate session recovery", () => {
  it("keeps the logged-out screen when an older login fails after logout", async () => {
    let releaseDiscovery!: () => void;
    let discoveryStarted!: () => void;
    const started = new Promise<void>((resolve) => {
      discoveryStarted = resolve;
    });
    const discoveryWait = new Promise<void>((resolve) => {
      releaseDiscovery = resolve;
    });
    const harness = createOidcHarness(async () => ({}), {
      wait: discoveryWait,
      onStart: discoveryStarted,
    });
    authMocks.client = harness.client;
    await renderGate();
    await started;

    let logout!: Promise<void>;
    await flush(() => {
      logout = harness.client.logout();
    });
    expect(container.textContent).toContain("auth.loggedOut");
    releaseDiscovery();
    await logout;
    await flush();

    expect(container.textContent).toContain("auth.loggedOut");
    expect(container.textContent).not.toContain("auth.failed");
    expect(harness.assigned).toHaveLength(1);
    expect(harness.assigned[0]).toContain("/logout");
    expect(takePending(sessionStorage, harness.now + 1)).toBeNull();
  });

  it("keeps a newer login navigation when logout discovery resolves later", async () => {
    let releaseDiscovery!: () => void;
    let discoveryStarted!: () => void;
    const started = new Promise<void>((resolve) => {
      discoveryStarted = resolve;
    });
    const discoveryWait = new Promise<void>((resolve) => {
      releaseDiscovery = resolve;
    });
    const harness = createOidcHarness(async () => ({}), {
      wait: discoveryWait,
      onStart: discoveryStarted,
    });
    authMocks.client = harness.client;
    await renderGate();
    await started;

    let logout!: Promise<void>;
    await flush(() => {
      logout = harness.client.logout();
    });
    expect(container.textContent).toContain("auth.loggedOut");
    await flush(() => {
      const login = Array.from(container.querySelectorAll("button")).find((button) =>
        button.textContent?.includes("auth.login"),
      );
      if (!login) throw new Error("login action was not rendered");
      login.click();
    });
    releaseDiscovery();
    await logout;
    await flush();

    await vi.waitFor(() => expect(harness.assigned).toHaveLength(1));
    expect(new URL(harness.assigned[0]!).pathname).toBe("/authorize");
    expect(container.textContent).toContain("auth-checking");
    expect(container.textContent).not.toContain("auth.loggedOut");
  });

  it("blocks logout navigation until storage cleanup can be retried", async () => {
    const storageGate = createSessionStorageGate();
    try {
      const harness = createOidcHarness(async (_body, nonce) => ({
        access_token: "access-1",
        refresh_token: "refresh-1",
        id_token: makeIdToken(nonce),
        expires_in: 300,
      }));
      await completeLogin(harness);
      harness.assigned.length = 0;
      authMocks.client = harness.client;
      await renderGate();
      expect(container.textContent).toContain("private-content");

      storageGate.deny();
      let logout!: Promise<void>;
      await flush(() => {
        logout = harness.client.logout();
      });
      await expect(logout).resolves.toBeUndefined();
      await flush();

      expect(harness.client.hasSession).toBe(false);
      expect(container.textContent).toContain("auth.logoutStorageFailed");
      expect(container.textContent).not.toContain("private-content");
      expect(harness.assigned).toHaveLength(0);

      storageGate.allow();
      await flush(() => {
        const retry = Array.from(container.querySelectorAll("button")).find((button) =>
          button.textContent?.includes("auth.retryLogout"),
        );
        if (!retry) throw new Error("logout cleanup retry action was not rendered");
        retry.click();
      });
      await vi.waitFor(() => expect(harness.assigned.at(-1)).toContain("/logout"));
      expect(container.textContent).not.toContain("private-content");
    } finally {
      storageGate.restore();
    }
  });

  it.each(["client construction", "first storage operation"] as const)(
    "shows a retryable screen when initial %s cannot read sessionStorage",
    async (failurePoint) => {
      const harness = createOidcHarness(async (_body, nonce) => ({
        access_token: "access-1",
        refresh_token: "refresh-1",
        id_token: makeIdToken(nonce),
        expires_in: 300,
      }));
      const storageDescriptor = Object.getOwnPropertyDescriptor(window, "sessionStorage");
      let clientFactoryCalls = 0;
      authMocks.clientFactory = () => {
        clientFactoryCalls += 1;
        if (failurePoint === "client construction" && clientFactoryCalls === 1) {
          // OidcClient's default browser environment reads this getter while
          // getOidcClient() is constructing the initial client.
          return new OidcClient({ issuer: ISSUER, clientId: "zoo" });
        }
        return harness.client;
      };
      Object.defineProperty(window, "sessionStorage", {
        configurable: true,
        get() {
          throw new DOMException("storage unavailable", "SecurityError");
        },
      });
      try {
        await renderGate();
        expect(container.textContent).toContain("auth.retry");
        expect(container.textContent).not.toContain("auth-checking");
        expect(harness.assigned).toHaveLength(0);

        if (storageDescriptor) Object.defineProperty(window, "sessionStorage", storageDescriptor);
        else Reflect.deleteProperty(window, "sessionStorage");
        await flush(() => {
          const retry = Array.from(container.querySelectorAll("button")).find((button) =>
            button.textContent?.includes("auth.retry"),
          );
          if (!retry) throw new Error("initialization retry action was not rendered");
          retry.click();
        });
        await vi.waitFor(() => expect(harness.assigned).toHaveLength(1));
        expect(clientFactoryCalls).toBe(2);
        expect(new URL(harness.assigned[0]!).pathname).toBe("/authorize");
      } finally {
        if (storageDescriptor) Object.defineProperty(window, "sessionStorage", storageDescriptor);
        else Reflect.deleteProperty(window, "sessionStorage");
      }
    },
  );

  it("shows a retryable state when reauthorization storage writes fail", async () => {
    const storageGate = createSessionStorageGate();
    try {
      const harness = createOidcHarness(async (_body, nonce) => ({
        access_token: "access-1",
        id_token: makeIdToken(nonce),
        expires_in: 10,
      }));
      await completeLogin(harness);
      authMocks.client = harness.client;
      await renderGate();
      storageGate.deny();
      await act(async () => {
        await expect(harness.client.getAccessToken()).resolves.toBe("access-1");
      });
      expect(container.textContent).toContain("auth.retry");
      storageGate.allow();
      await flush(() => {
        const retry = Array.from(container.querySelectorAll("button")).find((button) =>
          button.textContent?.includes("auth.retry"),
        );
        if (!retry) throw new Error("retry action missing");
        retry.click();
      });
      await vi.waitFor(() => expect(harness.assigned.at(-1)).toContain("/authorize"));
    } finally {
      storageGate.restore();
    }
  });

  it("does not turn a successful API response into a failure when clearing redirect attempts fails", async () => {
    const harness = createOidcHarness(async (_body, nonce) => ({
      access_token: "access-1",
      id_token: makeIdToken(nonce),
      expires_in: 300,
    }));
    await completeLogin(harness);
    const storageGate = createSessionStorageGate();
    try {
      authMocks.client = harness.client;
      await renderGate();
      const api = createApiClient({ fetch: async () => jsonResponse({ result: "server-saved" }) });
      storageGate.deny("removeItem");

      await expect(api.get("/api/notes")).resolves.toEqual({ result: "server-saved" });

      expect(container.textContent).toContain("private-content");
      expect(container.textContent).not.toContain("auth.failed");
      expect(harness.client.hasSession).toBe(true);
    } finally {
      storageGate.allow();
      storageGate.restore();
    }
  });

  it.each(["getItem", "setItem", "removeItem"] as const)(
    "resets the rejected session and leaves an authorization retry when redirect state %s fails",
    async (failurePoint) => {
      const harness = createOidcHarness(async (_body, nonce) => ({
        access_token: "access-1",
        refresh_token: "refresh-1",
        id_token: makeIdToken(nonce),
        expires_in: 300,
      }));
      await completeLogin(harness);
      const storageGate = createSessionStorageGate();
      try {
        authMocks.client = harness.client;
        await renderGate();
        const api = createApiClient({
          fetch: async () => new Response("", { status: 401 }),
        });
        if (failurePoint === "removeItem")
          sessionStorage.setItem("zoo.oidc.authorizeRedirectAt", "0");
        storageGate.deny(failurePoint);

        await act(async () => {
          await expect(api.get("/api/notes")).rejects.toMatchObject({ status: 401 });
          for (let tick = 0; tick < 8; tick += 1) await Promise.resolve();
        });

        expect(harness.client.hasSession).toBe(false);
        expect(container.textContent).toContain("auth.failed");
        expect(container.textContent).toContain("auth.retry");
        expect(container.textContent).not.toContain("private-content");

        storageGate.allow();
        await flush(() => {
          const retry = Array.from(container.querySelectorAll("button")).find((button) =>
            button.textContent?.includes("auth.retry"),
          );
          if (!retry) throw new Error("unauthorized storage recovery retry was not rendered");
          retry.click();
        });
        await vi.waitFor(() => expect(harness.assigned.at(-1)).toContain("/authorize"));
        expect(container.textContent).toContain("auth-checking");
      } finally {
        storageGate.allow();
        storageGate.restore();
      }
    },
  );

  it("retries draft recovery before starting authorization after storage recovers", async () => {
    let storageRecovered = false;
    let recoveryAttempts = 0;
    const unsubscribe = subscribeAuthSessionEnded((reason) => {
      if (reason !== "reauthorize") return;
      recoveryAttempts += 1;
      return storageRecovered;
    });
    try {
      const harness = createOidcHarness(async (_body, nonce) => ({
        access_token: "access-1",
        refresh_token: "refresh-1",
        id_token: makeIdToken(nonce),
        expires_in: 300,
      }));
      await completeLogin(harness);
      authMocks.client = harness.client;
      await renderGate();
      harness.assigned.length = 0;
      await flush(() => harness.client.resetSession());

      const clickRetry = async () =>
        flush(() => {
          const retry = Array.from(container.querySelectorAll("button")).find((button) =>
            button.textContent?.includes("auth.retry"),
          );
          if (!retry) throw new Error("retry action missing");
          retry.click();
        });
      await clickRetry();
      expect(recoveryAttempts).toBe(1);
      expect(harness.assigned).toHaveLength(0);
      expect(container.textContent).toContain("auth.retry");

      storageRecovered = true;
      await clickRetry();
      await vi.waitFor(() => expect(harness.assigned).toHaveLength(1));
      expect(recoveryAttempts).toBe(2);
      expect(new URL(harness.assigned[0]!).pathname).toBe("/authorize");
    } finally {
      unsubscribe();
    }
  });

  it("returns to a retryable error when the sessionStorage getter throws", async () => {
    const harness = createOidcHarness(async (_body, nonce) => ({
      access_token: "access-1",
      refresh_token: "refresh-1",
      id_token: makeIdToken(nonce),
      expires_in: 300,
    }));
    await completeLogin(harness);
    authMocks.client = harness.client;
    await renderGate();
    harness.assigned.length = 0;
    await flush(() => harness.client.resetSession());

    const storageDescriptor = Object.getOwnPropertyDescriptor(window, "sessionStorage");
    Object.defineProperty(window, "sessionStorage", {
      configurable: true,
      get() {
        throw new DOMException("storage unavailable", "SecurityError");
      },
    });
    try {
      await flush(() => {
        const retry = Array.from(container.querySelectorAll("button")).find((button) =>
          button.textContent?.includes("auth.retry"),
        );
        if (!retry) throw new Error("retry action missing");
        retry.click();
      });
      expect(container.textContent).toContain("auth.retry");
      expect(container.textContent).not.toContain("auth-checking");
      expect(harness.assigned).toHaveLength(0);
    } finally {
      if (storageDescriptor) Object.defineProperty(window, "sessionStorage", storageDescriptor);
      else Reflect.deleteProperty(window, "sessionStorage");
    }

    await flush(() => {
      const retry = Array.from(container.querySelectorAll("button")).find((button) =>
        button.textContent?.includes("auth.retry"),
      );
      if (!retry) throw new Error("retry action missing after storage recovery");
      retry.click();
    });
    await vi.waitFor(() => expect(harness.assigned).toHaveLength(1));
    expect(new URL(harness.assigned[0]!).pathname).toBe("/authorize");
  });

  it("ignores REST and Shape 401s started before a failed pre-expiry authorization and retries online", async () => {
    let rejectDiscovery!: (cause: unknown) => void;
    let discoveryStarted!: () => void;
    const reauthorizationDiscoveryStarted = new Promise<void>((resolve) => {
      discoveryStarted = resolve;
    });
    let tokenCallsStarted!: () => void;
    const bothTokenCallsStarted = new Promise<void>((resolve) => {
      tokenCallsStarted = resolve;
    });
    let releaseTokens!: () => void;
    const tokenGate = new Promise<void>((resolve) => {
      releaseTokens = resolve;
    });
    let apiFetchStarted!: () => void;
    const apiStarted = new Promise<void>((resolve) => {
      apiFetchStarted = resolve;
    });
    let shapeFetchStarted!: () => void;
    const shapeStarted = new Promise<void>((resolve) => {
      shapeFetchStarted = resolve;
    });
    let resolveApiResponse!: (response: Response) => void;
    let resolveShapeResponse!: (response: Response) => void;
    let tokenCalls = 0;
    let discoveryRequests = 0;
    const harness = createOidcHarness(
      async (_body, nonce) => ({
        access_token: "access-1",
        id_token: makeIdToken(nonce),
        expires_in: 10,
      }),
      {
        respond: async (request) => {
          discoveryRequests = request;
          if (request === 2) {
            discoveryStarted();
            return await new Promise<Response>((_resolve, reject) => {
              rejectDiscovery = reject;
            });
          }
          return jsonResponse({
            issuer: ISSUER,
            authorization_endpoint: `${ISSUER}/authorize`,
            token_endpoint: `${ISSUER}/token`,
            end_session_endpoint: `${ISSUER}/logout`,
          });
        },
      },
    );
    await completeLogin(harness);
    // Discovery is normally cached after the initial login. Resetting the cache
    // makes this controlled test exercise a pre-navigation discovery failure.
    (harness.client as unknown as { endpoints: unknown }).endpoints = null;
    authMocks.client = harness.client;
    await renderGate();
    harness.assigned.length = 0;
    harness.advance(1_000);
    const getToken = async () => {
      tokenCalls += 1;
      if (tokenCalls === 2) tokenCallsStarted();
      await tokenGate;
      return await harness.client.getAccessToken();
    };
    const apiClient = createApiClient({
      getToken,
      fetch: async () => {
        apiFetchStarted();
        return await new Promise<Response>((resolve) => {
          resolveApiResponse = resolve;
        });
      },
    });
    const shapeFetch = createShapeFetch({
      getToken,
      fetch: async () => {
        shapeFetchStarted();
        return await new Promise<Response>((resolve) => {
          resolveShapeResponse = resolve;
        });
      },
    });
    const apiRequest = apiClient.get("/api/notes");
    const shapeRequest = shapeFetch("https://api.example/v1/shape?table=notes");
    await bothTokenCallsStarted;

    await act(async () => {
      releaseTokens();
      await Promise.all([reauthorizationDiscoveryStarted, apiStarted, shapeStarted]);
      for (let tick = 0; tick < 8; tick += 1) await Promise.resolve();
    });
    expect(discoveryRequests).toBe(2);

    setOnline(false);
    await act(async () => {
      rejectDiscovery(new TypeError("discovery network failure"));
      for (let tick = 0; tick < 12; tick += 1) await Promise.resolve();
    });
    expect(container.textContent).toContain("discovery network failure");
    expect(sessionStorage.getItem("zoo.oidc.lastRedirectAt")).toBeNull();

    await act(async () => {
      resolveApiResponse(new Response("", { status: 401 }));
      resolveShapeResponse(new Response(null, { status: 401 }));
      await expect(apiRequest).rejects.toMatchObject({ status: 401 });
      await expect(shapeRequest).resolves.toMatchObject({ status: 401 });
      for (let tick = 0; tick < 12; tick += 1) await Promise.resolve();
    });
    expect(container.textContent).not.toContain("auth.unauthorizedLoop");

    setOnline(true);
    await flush(() => window.dispatchEvent(new Event("online")));
    await vi.waitFor(() => expect(harness.assigned).toHaveLength(1));
    expect(new URL(harness.assigned[0]!).pathname).toBe("/authorize");
    expect(container.textContent).toContain("auth-checking");
  });

  it("still stops a current 401 loop immediately after a completed redirect", async () => {
    const harness = createOidcHarness(async (_body, nonce) => ({
      access_token: "access-1",
      id_token: makeIdToken(nonce),
      expires_in: 300,
    }));
    await completeLogin(harness);
    authMocks.client = harness.client;
    await renderGate();
    harness.assigned.length = 0;
    markRedirectStarted(sessionStorage, Date.now());
    const apiClient = createApiClient({
      fetch: async () => new Response("", { status: 401 }),
    });

    await act(async () => {
      await expect(apiClient.get("/api/current-session")).rejects.toMatchObject({ status: 401 });
      for (let tick = 0; tick < 8; tick += 1) await Promise.resolve();
    });

    expect(container.textContent).toContain("auth.unauthorizedLoop");
    setOnline(false);
    setOnline(true);
    await flush(() => window.dispatchEvent(new Event("online")));
    expect(harness.assigned).toHaveLength(0);
    expect(container.textContent).toContain("auth.unauthorizedLoop");
  });

  it("shows a retryable state offline after refresh failure and resumes authorization online", async () => {
    let failRefresh!: (cause: unknown) => void;
    let refreshStarted!: () => void;
    const started = new Promise<void>((resolve) => {
      refreshStarted = resolve;
    });
    let authorizationExchanges = 0;
    const harness = createOidcHarness(async (body, nonce) => {
      if (body.get("grant_type") === "refresh_token") {
        refreshStarted();
        return await new Promise<unknown>((_resolve, reject) => {
          failRefresh = reject;
        });
      }
      authorizationExchanges += 1;
      return {
        access_token: `access-${authorizationExchanges}`,
        refresh_token: `refresh-${authorizationExchanges}`,
        id_token: makeIdToken(nonce),
        expires_in: authorizationExchanges === 1 ? 10 : 300,
      };
    });
    await completeLogin(harness);
    authMocks.client = harness.client;
    await renderGate();
    expect(container.textContent).toContain("private-content");
    harness.assigned.length = 0;
    harness.advance(11_000);
    setOnline(false);

    let accessToken: Promise<string | null>;
    await act(async () => {
      accessToken = harness.client.getAccessToken();
      await started;
      failRefresh(new TypeError("network unavailable"));
      await expect(accessToken!).resolves.toBeNull();
      for (let tick = 0; tick < 12; tick += 1) await Promise.resolve();
    });

    expect(container.textContent).not.toContain("private-content");
    expect(container.textContent).toContain("auth.retry");
    expect(harness.assigned).toHaveLength(0);

    setOnline(true);
    await flush(() => window.dispatchEvent(new Event("online")));
    await vi.waitFor(() => expect(harness.assigned).toHaveLength(1));
    const authorizeUrl = new URL(harness.assigned[0]!);
    expect(authorizeUrl.searchParams.get("response_type")).toBe("code");
    await flush(() => window.dispatchEvent(new Event("online")));
    expect(harness.assigned).toHaveLength(1);

    const pending = takePending(window.sessionStorage, harness.now + 1);
    if (!pending) throw new Error("relogin pending authorization was not stored");
    savePending(window.sessionStorage, pending);
    harness.setNonce(pending.nonce);
    harness.setHref(`${CALLBACK}?code=relogin-code&state=${pending.state}`);
    await flush(() => root.unmount());
    root = createRoot(container);
    await renderGate();

    expect(container.textContent).toContain("private-content");
    expect(harness.client.hasSession).toBe(true);
    await expect(harness.client.getAccessToken()).resolves.toBe("access-2");
  });

  it("retries pre-expiry authorization online with a live token and keeps explicit logout", async () => {
    let authorizationExchanges = 0;
    const harness = createOidcHarness(async (_body, nonce) => {
      authorizationExchanges += 1;
      return {
        access_token: `access-${authorizationExchanges}`,
        id_token: makeIdToken(nonce),
        expires_in: 10,
      };
    });
    await completeLogin(harness);
    authMocks.client = harness.client;
    await renderGate();
    expect(container.textContent).toContain("private-content");
    harness.assigned.length = 0;
    harness.advance(1_000);
    setOnline(false);

    await act(async () => {
      await expect(harness.client.getAccessToken()).resolves.toBe("access-1");
      for (let tick = 0; tick < 12; tick += 1) await Promise.resolve();
    });
    expect(harness.client.hasSession).toBe(true);
    expect(container.textContent).toContain("auth.retry");
    expect(harness.assigned).toHaveLength(0);

    setOnline(true);
    await flush(() => window.dispatchEvent(new Event("online")));
    await vi.waitFor(() => expect(harness.assigned).toHaveLength(1));
    expect(new URL(harness.assigned[0]!).pathname).toBe("/authorize");
    await flush(() => window.dispatchEvent(new Event("online")));
    expect(harness.assigned).toHaveLength(1);

    await flush(() => harness.client.logout());
    await vi.waitFor(() => expect(harness.assigned).toHaveLength(2));
    expect(new URL(harness.assigned[1]!).pathname).toBe("/logout");
    expect(container.textContent).toContain("auth.loggedOut");
    await flush(() => window.dispatchEvent(new Event("online")));
    expect(harness.assigned).toHaveLength(2);
    expect(container.textContent).toContain("auth.loggedOut");
  });

  it("keeps the logged-out screen when an old authorization callback resolves after logout", async () => {
    let finishCodeExchange!: () => void;
    let codeExchangeStarted!: () => void;
    const started = new Promise<void>((resolve) => {
      codeExchangeStarted = resolve;
    });
    const gate = new Promise<void>((resolve) => {
      finishCodeExchange = resolve;
    });
    const harness = createOidcHarness(async (_body, nonce) => {
      codeExchangeStarted();
      await gate;
      return {
        access_token: "old-session-token",
        refresh_token: "old-session-refresh",
        id_token: makeIdToken(nonce),
        expires_in: 300,
      };
    });
    await harness.client.login();
    const pending = takePending(window.sessionStorage, harness.now + 1);
    if (!pending) throw new Error("pending authorization was not stored");
    savePending(window.sessionStorage, pending);
    harness.setNonce(pending.nonce);
    harness.setHref(`${CALLBACK}?code=old-session-code&state=${pending.state}`);
    authMocks.client = harness.client;
    await renderGate();
    await started;

    await act(async () => {
      await harness.client.logout();
      for (let tick = 0; tick < 8; tick += 1) await Promise.resolve();
    });
    expect(container.textContent).toContain("auth.loggedOut");
    finishCodeExchange();
    await flush();

    expect(container.textContent).toContain("auth.loggedOut");
    expect(container.textContent).not.toContain("auth.failed");
    expect(authMocks.router.history.replace).not.toHaveBeenCalled();
    expect(harness.client.hasSession).toBe(false);
  });

  it("does not let a delayed API 401 from before logout clear the logout state", async () => {
    const harness = createOidcHarness(async (_body, nonce) => ({
      access_token: "session-access-token",
      refresh_token: "session-refresh-token",
      id_token: makeIdToken(nonce),
      expires_in: 300,
    }));
    await completeLogin(harness);
    authMocks.client = harness.client;
    await renderGate();
    expect(container.textContent).toContain("private-content");
    harness.assigned.length = 0;

    let releaseResponse!: (response: Response) => void;
    let requestStarted!: () => void;
    const started = new Promise<void>((resolve) => {
      requestStarted = resolve;
    });
    const apiFetch: FetchLike = () => {
      requestStarted();
      return new Promise((resolve) => {
        releaseResponse = resolve;
      });
    };
    const api = createApiClient({ fetch: apiFetch });
    const oldRequest = api.get("/api/users/me");
    await started;

    await act(async () => {
      await harness.client.logout();
      for (let tick = 0; tick < 8; tick += 1) await Promise.resolve();
    });
    expect(container.textContent).toContain("auth.loggedOut");
    await act(async () => {
      releaseResponse(new Response("", { status: 401 }));
      await expect(oldRequest).rejects.toMatchObject({ status: 401 });
      for (let tick = 0; tick < 8; tick += 1) await Promise.resolve();
    });

    expect(container.textContent).toContain("auth.loggedOut");
    expect(isLoggedOut(window.sessionStorage)).toBe(true);
    expect(harness.assigned).toHaveLength(1);
    expect(new URL(harness.assigned[0]!).pathname).toBe("/logout");
  });
});
