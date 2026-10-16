export interface JwtPayload {
  iss?: string;
  aud?: string | string[];
  exp?: number;
  nonce?: string;
  [claim: string]: unknown;
}

/**
 * JWT のペイロード部をデコードする（署名検証はしない）。
 *
 * 署名検証は Rust（唯一の認証ゲート）の責務。ここでは ID トークンを取り違え
 * 検知のために読むだけで、ID トークンはトークンエンドポイントから TLS 越しに
 * 直接受け取るため、OIDC Core 3.1.3.7 が認める「TLS サーバ検証」で足りる。
 */
export function decodeJwtPayload(token: string): JwtPayload | null {
  const parts = token.split(".");
  if (parts.length !== 3 || !parts[1]) {
    return null;
  }
  try {
    const normalized = parts[1].replace(/-/g, "+").replace(/_/g, "/");
    const padded = normalized.padEnd(normalized.length + ((4 - (normalized.length % 4)) % 4), "=");
    const binary = atob(padded);
    const bytes = Uint8Array.from(binary, (char) => char.charCodeAt(0));
    const parsed: unknown = JSON.parse(new TextDecoder().decode(bytes));
    if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
      return null;
    }
    return parsed as JwtPayload;
  } catch {
    return null;
  }
}

export function audienceIncludes(aud: string | string[] | undefined, clientId: string): boolean {
  if (typeof aud === "string") {
    return aud === clientId;
  }
  return Array.isArray(aud) && aud.includes(clientId);
}

export function issuerMatches(iss: string | undefined, issuer: string): boolean {
  if (!iss) {
    return false;
  }
  return iss.replace(/\/+$/, "") === issuer.replace(/\/+$/, "");
}
