import type { AppConfig } from "../config";
import type { KeySlot } from "@zoo/crypto-wasm";

/**
 * パスキー（WebAuthn PRF）で SSSS の鍵スロットを作り、開ける（docs/e2ee.md の「パスキーと Related
 * Origin Requests」。#360）。
 *
 * - RP ID は AuthZ のホスト（OIDC の issuer のホスト名。本番は `account.zli.works`）。zoo2 とは
 *   登録可能ドメインが違うので、ブラウザーは Related Origin Requests で AuthZ の
 *   `/.well-known/webauthn` を確かめる。
 * - アサーションはどこにも送らない。目的は PRF 出力だけ。
 */

export type PasskeyErrorCode =
  /** 利用者が取り消した、または使えるパスキーがなかった（ブラウザーは区別させない）。 */
  | "cancelled"
  /** 認証器かブラウザーが PRF に対応していない。 */
  | "prfUnsupported"
  /** このオリジンから AuthZ の RP ID を使えない（Related Origin Requests の許可がない）。 */
  | "originNotAllowed"
  /** 返ったパスキーに対応するスロットがない。 */
  | "noSlot"
  | "unknown";

export class PasskeyError extends Error {
  constructor(
    readonly code: PasskeyErrorCode,
    message: string,
  ) {
    super(message);
    this.name = "PasskeyError";
  }
}

export interface PrfResult {
  /** base64url（パディングなし）の `credential.rawId`。 */
  credentialId: string;
  prfOutput: Uint8Array;
}

/** 呼び出し側から差し替えられるように、`navigator.credentials.get` を渡す。 */
export type CredentialGetter = (options: CredentialRequestOptions) => Promise<Credential | null>;

const defaultGetter: CredentialGetter = (options) => navigator.credentials.get(options);

/** issuer のホスト名を RP ID にする。OIDC を設定していなければ `null`（リカバリーキーだけを使う）。 */
export function passkeyRpId(config: Pick<AppConfig, "oidc">): string | null {
  if (!config.oidc) return null;
  try {
    return new URL(config.oidc.issuer).hostname;
  } catch {
    return null;
  }
}

/**
 * このブラウザーでパスキー（PRF）を試せるか。PRF に対応しないと分かっているときだけ `false` を返す。
 * 対応が分からない（`getClientCapabilities` がない）ときは試して、結果で判断する。
 */
export async function passkeyPrfSupported(): Promise<boolean> {
  if (typeof window === "undefined" || typeof window.PublicKeyCredential === "undefined") {
    return false;
  }
  const getCapabilities = (
    window.PublicKeyCredential as unknown as {
      getClientCapabilities?: () => Promise<Record<string, boolean | undefined>>;
    }
  ).getClientCapabilities;
  if (!getCapabilities) return true;
  try {
    const capabilities = await getCapabilities.call(window.PublicKeyCredential);
    return capabilities["extension:prf"] !== false;
  } catch {
    return true;
  }
}

export function base64UrlEncode(bytes: Uint8Array): string {
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

export function base64UrlDecode(value: string): Uint8Array<ArrayBuffer> {
  return base64Decode(value.replace(/-/g, "+").replace(/_/g, "/"));
}

/** パディングの有無を問わない base64。 */
export function base64Decode(value: string): Uint8Array<ArrayBuffer> {
  const padded = value + "=".repeat((4 - (value.length % 4)) % 4);
  const binary = atob(padded);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) bytes[index] = binary.charCodeAt(index);
  return bytes;
}

function randomChallenge(): Uint8Array<ArrayBuffer> {
  return crypto.getRandomValues(new Uint8Array(32));
}

interface PrfExtensionResults {
  prf?: { enabled?: boolean; results?: { first?: unknown } };
}

/**
 * PRF の結果（`results.first`）をバイト列にする。32 バイトでなければ `null`。
 *
 * 仕様では `ArrayBuffer` か `ArrayBufferView` だが、次の形も読む。
 * - 拡張機能が別の実行環境（realm）で作った `ArrayBuffer`。`instanceof ArrayBuffer` では見分けられない
 *   ので、`ArrayBuffer.isView` と型のタグで見分ける。
 * - 0〜255 の整数の配列。1Password のブラウザー拡張（Chrome、Firefox）が返す、仕様に合わない形
 *   （1Password の課題 FS-5593）。
 */
export function prfBytes(value: unknown): Uint8Array | null {
  let bytes: Uint8Array;
  if (ArrayBuffer.isView(value)) {
    bytes = new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
  } else if (Object.prototype.toString.call(value) === "[object ArrayBuffer]") {
    bytes = new Uint8Array(value as ArrayBuffer);
  } else if (
    Array.isArray(value) &&
    value.every((byte) => Number.isInteger(byte) && byte >= 0 && byte <= 255)
  ) {
    bytes = Uint8Array.from(value as number[]);
  } else {
    return null;
  }
  // 呼び出し側の元のバッファーを共有しないよう、写しを返す。
  return bytes.length === 32 ? new Uint8Array(bytes) : null;
}

function prfFirst(credential: PublicKeyCredential): Uint8Array | null {
  const results = credential.getClientExtensionResults() as PrfExtensionResults;
  return prfBytes(results.prf?.results?.first);
}

function toPasskeyError(error: unknown): PasskeyError {
  if (error instanceof PasskeyError) return error;
  const name = error instanceof Error ? error.name : "";
  const message = error instanceof Error ? error.message : String(error);
  if (name === "NotAllowedError" || name === "AbortError")
    return new PasskeyError("cancelled", message);
  if (name === "SecurityError") return new PasskeyError("originNotAllowed", message);
  if (name === "NotSupportedError") return new PasskeyError("prfUnsupported", message);
  return new PasskeyError("unknown", message);
}

async function getCredential(
  getter: CredentialGetter,
  publicKey: PublicKeyCredentialRequestOptions,
): Promise<PublicKeyCredential> {
  let credential: Credential | null;
  try {
    credential = await getter({ publicKey });
  } catch (error) {
    throw toPasskeyError(error);
  }
  if (!credential || credential.type !== "public-key") {
    throw new PasskeyError("cancelled", "パスキーが選ばれませんでした");
  }
  return credential as PublicKeyCredential;
}

/**
 * 鍵スロットを足すために、利用者が選んだパスキーで PRF を評価する（`allowCredentials` は空）。
 * resident key でないパスキー（YubiKey など）は選べないので、`cancelled` になる。
 */
export async function evaluatePrfForNewSlot(
  rpId: string,
  prfSalt: Uint8Array<ArrayBuffer>,
  getter: CredentialGetter = defaultGetter,
): Promise<PrfResult> {
  const credential = await getCredential(getter, {
    rpId,
    challenge: randomChallenge(),
    userVerification: "required",
    allowCredentials: [],
    extensions: { prf: { eval: { first: prfSalt } } } as AuthenticationExtensionsClientInputs,
  });
  const prfOutput = prfFirst(credential);
  if (!prfOutput) throw new PasskeyError("prfUnsupported", "PRF の結果がありません");
  return { credentialId: base64UrlEncode(new Uint8Array(credential.rawId)), prfOutput };
}

/** 既存のパスキーのスロットで PRF を評価する（解錠）。使ったスロットも返す。 */
export async function evaluatePrfForUnlock(
  rpId: string,
  slots: KeySlot[],
  getter: CredentialGetter = defaultGetter,
): Promise<PrfResult & { slot: KeySlot }> {
  const passkeySlots = slots.filter(
    (slot): slot is KeySlot & { credential_id: string; prf_salt: string } =>
      slot.kind === "passkey" && !!slot.credential_id && !!slot.prf_salt,
  );
  if (passkeySlots.length === 0) throw new PasskeyError("noSlot", "パスキーのスロットがありません");
  const credential = await getCredential(getter, {
    rpId,
    challenge: randomChallenge(),
    userVerification: "required",
    allowCredentials: passkeySlots.map((slot) => ({
      type: "public-key" as const,
      id: base64UrlDecode(slot.credential_id),
    })),
    extensions: {
      prf: {
        evalByCredential: Object.fromEntries(
          passkeySlots.map((slot) => [slot.credential_id, { first: base64Decode(slot.prf_salt) }]),
        ),
      },
    } as AuthenticationExtensionsClientInputs,
  });
  const credentialId = base64UrlEncode(new Uint8Array(credential.rawId));
  const slot = passkeySlots.find((candidate) => candidate.credential_id === credentialId);
  if (!slot) throw new PasskeyError("noSlot", "選んだパスキーのスロットがありません");
  const prfOutput = prfFirst(credential);
  if (!prfOutput) throw new PasskeyError("prfUnsupported", "PRF の結果がありません");
  return { credentialId, prfOutput, slot };
}
