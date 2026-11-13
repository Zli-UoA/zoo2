import { describe, expect, it, vi } from "vite-plus/test";

import type { ReauthRequest } from "../oidc/client";
import {
  REAUTH_CALLBACK_PATH,
  ReauthError,
  reauthenticate,
  type ReauthEnvironment,
} from "./reauth";

const ORIGIN = "https://zli.chat";

function environment() {
  let listener: ((event: MessageEvent) => void) | null = null;
  const channel = { onmessage: null as ((event: MessageEvent) => void) | null, close: vi.fn() };
  let timeout: (() => void) | null = null;
  const env: ReauthEnvironment = {
    origin: ORIGIN,
    onMessage(next) {
      listener = next;
      return () => {
        listener = null;
      };
    },
    openChannel: () => channel,
    setTimeout(callback) {
      timeout = callback;
      return () => {
        timeout = null;
      };
    },
  };
  return {
    env,
    channel,
    post: (data: unknown, origin = ORIGIN) => listener?.({ data, origin } as MessageEvent),
    expire: () => timeout?.(),
  };
}

function oidc() {
  const request: ReauthRequest = {
    url: "https://account.zli.works/api/auth/oauth2/authorize?max_age=0",
    state: "state-1",
    nonce: "nonce-1",
    verifier: "verifier-1",
    redirectUri: `${ORIGIN}${REAUTH_CALLBACK_PATH}`,
  };
  return {
    request,
    client: {
      createReauthRequest: vi.fn(async () => request),
      exchangeReauthCode: vi.fn(async () => "id-token"),
    },
  };
}

function popup() {
  return { location: { href: "about:blank" }, close: vi.fn() } as unknown as Window & {
    close: ReturnType<typeof vi.fn>;
  };
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("reauthenticate", () => {
  it("別ウィンドウで max_age=0 を開き、同じオリジンと state の認可コードだけを受け取る", async () => {
    const { env, post } = environment();
    const { client, request } = oidc();
    const window = popup();
    const result = reauthenticate({ oidc: client, popup: window, environment: env });
    await flush();
    expect(client.createReauthRequest).toHaveBeenCalledWith(`${ORIGIN}${REAUTH_CALLBACK_PATH}`);
    expect(window.location.href).toBe(request.url);
    // 別のオリジンと、違う state は無視する。
    post({ type: "zoo-reauth", code: "evil", state: "state-1", error: null }, "https://evil.test");
    post({ type: "zoo-reauth", code: "other", state: "state-2", error: null });
    post({ type: "zoo-reauth", code: "code-1", state: "state-1", error: null });
    await expect(result).resolves.toBe("id-token");
    expect(client.exchangeReauthCode).toHaveBeenCalledWith(request, "code-1");
    expect(window.close).toHaveBeenCalled();
  });

  it("opener が切れても、BroadcastChannel で受け取れる", async () => {
    const { env, channel } = environment();
    const { client } = oidc();
    const result = reauthenticate({ oidc: client, popup: popup(), environment: env });
    await flush();
    channel.onmessage?.({
      data: { type: "zoo-reauth", code: "code-1", state: "state-1", error: null },
    } as MessageEvent);
    await expect(result).resolves.toBe("id-token");
    expect(channel.close).toHaveBeenCalled();
  });

  it("ポップアップを止められた、取り消された、時間切れを区別する", async () => {
    const { client } = oidc();
    await expect(reauthenticate({ oidc: client, popup: null })).rejects.toMatchObject({
      code: "popupBlocked",
    });

    const denied = environment();
    const deniedResult = reauthenticate({
      oidc: client,
      popup: popup(),
      environment: denied.env,
    });
    await flush();
    denied.post({ type: "zoo-reauth", code: null, state: "state-1", error: "access_denied" });
    await expect(deniedResult).rejects.toMatchObject({ code: "denied" });

    const expired = environment();
    const expiredResult = reauthenticate({
      oidc: client,
      popup: popup(),
      environment: expired.env,
    });
    await flush();
    expired.expire();
    await expect(expiredResult).rejects.toBeInstanceOf(ReauthError);
    await expect(expiredResult).rejects.toMatchObject({ code: "timeout" });
  });
});
