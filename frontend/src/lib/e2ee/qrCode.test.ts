import { describe, expect, it } from "vite-plus/test";

import {
  bytesFromDetected,
  bytesToLatin1,
  decodeQrImage,
  latin1ToBytes,
  looksLikeVerificationQr,
  qrModules,
} from "./qrCode";

const TXN = "6f1c2b3a-4d5e-4f60-8a7b-9c0d1e2f3a4b";

/** 検証の QR のバイト列（Matrix の形）。鍵と秘密には 0x80〜0xFF のバイトも入れる。 */
function verificationQr(): Uint8Array {
  return Uint8Array.from([
    ...Array.from("MATRIX", (c) => c.charCodeAt(0)),
    0x02,
    0x00,
    0x00,
    TXN.length,
    ...Array.from(TXN, (c) => c.charCodeAt(0)),
    ...Array.from({ length: 64 }, (_, i) => (i * 37 + 0x80) & 0xff),
    ...Array.from({ length: 16 }, (_, i) => 0xff - i),
  ]);
}

/** QR のモジュールを、白地に黒の RGBA の画像にする（1 モジュール = `scale` 画素、周りに 4 モジュールの余白）。 */
function render(modules: boolean[][], scale = 4) {
  const size = (modules.length + 8) * scale;
  const data = new Uint8ClampedArray(size * size * 4).fill(255);
  modules.forEach((row, y) =>
    row.forEach((dark, x) => {
      if (!dark) return;
      for (let dy = 0; dy < scale; dy += 1) {
        for (let dx = 0; dx < scale; dx += 1) {
          const offset = (((y + 4) * scale + dy) * size + (x + 4) * scale + dx) * 4;
          data.fill(0, offset, offset + 3);
        }
      }
    }),
  );
  return { data, size };
}

describe("検証の QR", () => {
  it("作った QR を JS のデコーダーで読むと、同じバイト列に戻る", () => {
    const bytes = verificationQr();
    const { data, size } = render(qrModules(bytes));
    expect(decodeQrImage(data, size, size)).toEqual(bytes);
  });

  it("QR のない画像では null", () => {
    const data = new Uint8ClampedArray(64 * 64 * 4).fill(255);
    expect(decodeQrImage(data, 64, 64)).toBeNull();
  });

  it("Latin-1 の文字列とバイト列を行き来できる。255 を超える文字があれば戻さない", () => {
    const bytes = verificationQr();
    expect(latin1ToBytes(bytesToLatin1(bytes))).toEqual(bytes);
    expect(latin1ToBytes("€")).toBeNull();
  });

  it("形の合わないものは検証の QR と見なさない", () => {
    const bytes = verificationQr();
    expect(looksLikeVerificationQr(bytes)).toBe(true);
    expect(looksLikeVerificationQr(bytes.slice(0, bytes.length - 9))).toBe(false);
    const otherMode = bytes.slice();
    otherMode[7] = 0x01;
    expect(looksLikeVerificationQr(otherMode)).toBe(false);
    expect(looksLikeVerificationQr(new TextEncoder().encode("https://example.com"))).toBe(false);
  });

  it("BarcodeDetector の rawValue は、Latin-1 として戻せて形が合うものだけを使う", () => {
    const bytes = verificationQr();
    expect(bytesFromDetected([{ rawValue: bytesToLatin1(bytes) }])).toEqual(bytes);
    // UTF-8 として読まれた（0x80 以上のバイトが置換文字になった）ものは使わない。
    const utf8 = new TextDecoder().decode(bytes);
    expect(bytesFromDetected([{ rawValue: utf8 }])).toBeNull();
    // ほかの QR と並んでいても、検証の QR を選ぶ。
    expect(
      bytesFromDetected([{ rawValue: "https://example.com" }, { rawValue: bytesToLatin1(bytes) }]),
    ).toEqual(bytes);
  });
});
