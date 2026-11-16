/**
 * テストのサーバーが、鍵バックアップのアップロードの証明（docs/e2ee.md の「アップロードの証明」）を
 * 確かめるための道具。zoo-crypto（wasm）を使わず、WebCrypto の X25519 と HKDF で別に計算する。
 *
 * サーバーの upload key の秘密鍵は 0x22 の 32 バイト（`crates/zoo-crypto` の照合用の値と同じ）。
 */

/** upload key の公開鍵（0x22 の 32 バイトの秘密鍵から作ったもの。パディングなしの base64）。 */
export const TEST_UPLOAD_KEY = "D6poTtKIZ7l/Smot7l34zpdOdrcBjj8iocTPJnhXDyA";
/** 別の upload key の公開鍵（0x11 の 32 バイトの秘密鍵から作ったもの）。証明が合わない場合に使う。 */
export const OTHER_UPLOAD_KEY = "e06Qm75//kTEZaIgA31gjuNYl9Me+XLwf3SJLLD3PxM";

const PROOF_INFO = "zoo e2ee megolm backup upload v1";

function base64Url(bytes: Uint8Array): string {
  return btoa(String.fromCharCode(...bytes))
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=+$/, "");
}

function fromBase64(value: string): Uint8Array<ArrayBuffer> {
  return Uint8Array.from(atob(value), (c) => c.charCodeAt(0));
}

function toBase64(bytes: Uint8Array): string {
  return btoa(String.fromCharCode(...bytes)).replace(/=+$/, "");
}

function lengthPrefixed(parts: string[]): Uint8Array<ArrayBuffer> {
  const encoded = parts.map((part) => new TextEncoder().encode(part));
  const out = new Uint8Array(encoded.reduce((n, part) => n + 4 + part.length, 0));
  const view = new DataView(out.buffer);
  let offset = 0;
  for (const part of encoded) {
    view.setUint32(offset, part.length);
    out.set(part, offset + 4);
    offset += 4 + part.length;
  }
  return out;
}

/**
 * サーバーが作る証明（`X25519(upload key の秘密鍵, auth_data.public_key)` から HKDF）。`backupPublicKey` は
 * 版の `auth_data.public_key`。
 */
export async function expectedUploadProof(
  backupPublicKey: string,
  userId: string,
  version: string,
): Promise<string> {
  const secret = new Uint8Array(32).fill(0x22);
  const privateKey = await crypto.subtle.importKey(
    "jwk",
    {
      kty: "OKP",
      crv: "X25519",
      d: base64Url(secret),
      x: base64Url(fromBase64(TEST_UPLOAD_KEY)),
    },
    { name: "X25519" },
    false,
    ["deriveBits"],
  );
  const publicKey = await crypto.subtle.importKey(
    "raw",
    fromBase64(backupPublicKey),
    { name: "X25519" },
    false,
    [],
  );
  const shared = await crypto.subtle.deriveBits(
    { name: "X25519", public: publicKey },
    privateKey,
    256,
  );
  const ikm = await crypto.subtle.importKey("raw", shared, "HKDF", false, ["deriveBits"]);
  const proof = await crypto.subtle.deriveBits(
    {
      name: "HKDF",
      hash: "SHA-256",
      salt: new Uint8Array(32),
      info: lengthPrefixed([PROOF_INFO, userId, version]),
    },
    ikm,
    256,
  );
  return toBase64(new Uint8Array(proof));
}
