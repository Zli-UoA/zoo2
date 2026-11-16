import jsQR from "jsqr";
import qrcode from "qrcode-generator";

/**
 * QR コードによる検証（docs/e2ee.md の「QR コードによる検証」）の QR の描画と読み取り。中身の作成と
 * 検証は `zoo-crypto`（`SasVerification::qr_code`、`scan_qr_code`）が行い、ここはバイト列と画像の
 * 変換だけを持つ。
 */

/** QR の先頭（Matrix と同じ `MATRIX`）。 */
const PREFIX = [0x4d, 0x41, 0x54, 0x52, 0x49, 0x58];
/** 先頭、版、方式、`transaction_id` の長さ（2 バイト）。 */
const HEADER_LEN = PREFIX.length + 4;
/** 2 つのマスター鍵と、共有の秘密の最小の長さ。 */
const KEYS_AND_SECRET_MIN_LEN = 32 + 32 + 8;

/** バイト列を、1 文字が 1 バイト（0〜255）の文字列にする。 */
export function bytesToLatin1(bytes: ArrayLike<number>): string {
  let text = "";
  for (let i = 0; i < bytes.length; i += 1) text += String.fromCharCode(bytes[i] & 0xff);
  return text;
}

/** 1 文字が 1 バイトの文字列をバイト列に戻す。255 を超える文字があれば（別の文字コードで読まれた）`null`。 */
export function latin1ToBytes(text: string): Uint8Array | null {
  const bytes = new Uint8Array(text.length);
  for (let i = 0; i < text.length; i += 1) {
    const code = text.charCodeAt(i);
    if (code > 0xff) return null;
    bytes[i] = code;
  }
  return bytes;
}

/**
 * 検証の QR の形（先頭、版、長さ）をしているか。`BarcodeDetector` の `rawValue` から戻したバイト列が
 * 壊れていないかを、暗号の検証の前に見分けるためだけに使う（鍵と秘密の検証は `zoo-crypto` が行う）。
 */
export function looksLikeVerificationQr(bytes: Uint8Array): boolean {
  if (bytes.length < HEADER_LEN + KEYS_AND_SECRET_MIN_LEN) return false;
  if (PREFIX.some((byte, i) => bytes[i] !== byte)) return false;
  if (bytes[6] !== 0x02 || bytes[7] !== 0x00) return false;
  const txnLen = (bytes[8] << 8) | bytes[9];
  return bytes.length >= HEADER_LEN + txnLen + KEYS_AND_SECRET_MIN_LEN;
}

/** QR のモジュール（黒なら `true`）。バイトモード、誤り訂正は M（docs/e2ee.md）。 */
export function qrModules(bytes: ArrayLike<number>): boolean[][] {
  const qr = qrcode(0, "M");
  // 既定の stringToBytes は 1 文字を下位 8 ビットのバイトにするので、バイト列がそのまま入る。
  qr.addData(bytesToLatin1(bytes), "Byte");
  qr.make();
  const size = qr.getModuleCount();
  return Array.from({ length: size }, (_, row) =>
    Array.from({ length: size }, (_, col) => qr.isDark(row, col)),
  );
}

/** 画像（RGBA）から QR を読み、バイト列を返す（JS のデコーダー）。見つからなければ `null`。 */
export function decodeQrImage(
  data: Uint8ClampedArray,
  width: number,
  height: number,
): Uint8Array | null {
  const found = jsQR(data, width, height, { inversionAttempts: "attemptBoth" });
  return found ? Uint8Array.from(found.binaryData) : null;
}

/** `BarcodeDetector` の最小限の型（TypeScript の DOM の型にまだない）。 */
export interface BarcodeDetectorLike {
  detect(source: CanvasImageSource): Promise<{ rawValue: string }[]>;
}

interface BarcodeDetectorConstructor {
  new (options: { formats: string[] }): BarcodeDetectorLike;
  getSupportedFormats?: () => Promise<string[]>;
}

/**
 * QR を読める `BarcodeDetector` を作る。ない（Firefox、PC の Safari）か、QR に対応していなければ `null`
 * （JS のデコーダーで読む）。
 */
export async function createBarcodeDetector(): Promise<BarcodeDetectorLike | null> {
  const ctor = (globalThis as { BarcodeDetector?: BarcodeDetectorConstructor }).BarcodeDetector;
  if (!ctor) return null;
  try {
    const formats = (await ctor.getSupportedFormats?.()) ?? ["qr_code"];
    if (!formats.includes("qr_code")) return null;
    return new ctor({ formats: ["qr_code"] });
  } catch {
    return null;
  }
}

/**
 * `BarcodeDetector` の結果から、検証の QR のバイト列を取り出す。`rawValue` は文字列なので、Latin-1 として
 * バイト列に戻せないもの（UTF-8 や Shift_JIS として読まれた）や、形が合わないものは `null`（同じ画像を
 * JS のデコーダーで読み直す）。
 */
export function bytesFromDetected(results: { rawValue: string }[]): Uint8Array | null {
  for (const result of results) {
    const bytes = latin1ToBytes(result.rawValue);
    if (bytes && looksLikeVerificationQr(bytes)) return bytes;
  }
  return null;
}
