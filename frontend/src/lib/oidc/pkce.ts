/** RFC 7636 Appendix A: code_verifier に使える URL-safe な文字集合。 */
const UNRESERVED_ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";

export function base64UrlEncode(bytes: Uint8Array): string {
  let binary = "";
  for (const byte of bytes) {
    binary += String.fromCharCode(byte);
  }
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

export function randomBytes(length: number): Uint8Array {
  const bytes = new Uint8Array(length);
  globalThis.crypto.getRandomValues(bytes);
  return bytes;
}

/**
 * PKCE の code_verifier（43〜128 文字の URL-safe 文字列）を生成する。
 * 32 バイトの乱数を base64url で符号化するため 43 文字になり、最低長を満たす。
 */
export function createVerifier(): string {
  return base64UrlEncode(randomBytes(32));
}

/** code_verifier の SHA-256 を base64url で符号化した code_challenge (S256)。 */
export async function createChallenge(verifier: string): Promise<string> {
  const digest = await globalThis.crypto.subtle.digest(
    "SHA-256",
    new TextEncoder().encode(verifier),
  );
  return base64UrlEncode(new Uint8Array(digest));
}

/** authorize リクエストの state（CSRF 対策）。 */
export function createState(): string {
  return base64UrlEncode(randomBytes(16));
}

/** ID トークンの nonce（リプレイ対策）。 */
export function createNonce(): string {
  return base64UrlEncode(randomBytes(16));
}

export interface PkcePair {
  verifier: string;
  challenge: string;
  method: "S256";
}

export async function createPkcePair(): Promise<PkcePair> {
  const verifier = createVerifier();
  return { verifier, challenge: await createChallenge(verifier), method: "S256" };
}

/** verifier に不正な文字が混ざっていないか（テスト・防御的な検証用）。 */
export function isUnreserved(value: string): boolean {
  if (value.length === 0) {
    return false;
  }
  for (let index = 0; index < value.length; index += 1) {
    if (!UNRESERVED_ALPHABET.includes(value.charAt(index))) {
      return false;
    }
  }
  return true;
}
