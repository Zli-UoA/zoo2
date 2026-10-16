import type { AppConfig } from "../config";
import { OidcClient } from "./client";

let client: OidcClient | null = null;
let currentKey: string | null = null;

/** 設定が同じ間は同一クライアント（＝同一メモリ内セッション）を使い回す。 */
export function getOidcClient(config: AppConfig): OidcClient | null {
  const oidc = config.oidc;
  if (!oidc) {
    return null;
  }
  const key = JSON.stringify(oidc);
  if (client && currentKey === key) {
    return client;
  }
  client = new OidcClient({
    issuer: oidc.issuer,
    clientId: oidc.clientId,
    redirectUri: oidc.redirectUri,
    scope: oidc.scope,
    postLogoutRedirectUri: oidc.postLogoutRedirectUri,
  });
  currentKey = key;
  return client;
}

export function resetOidcClient(): void {
  client = null;
  currentKey = null;
}
