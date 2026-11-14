import { describe, expect, it } from "vite-plus/test";

import { prfBytes } from "./passkey";

const OUTPUT = Array.from({ length: 32 }, (_, index) => index * 7);

describe("prfBytes", () => {
  it("ArrayBuffer を読む", () => {
    expect(prfBytes(Uint8Array.from(OUTPUT).buffer)).toEqual(Uint8Array.from(OUTPUT));
  });

  it("ArrayBufferView を、オフセットと長さを守って読む", () => {
    const backing = new Uint8Array(40);
    backing.set(OUTPUT, 4);
    expect(prfBytes(new Uint8Array(backing.buffer, 4, 32))).toEqual(Uint8Array.from(OUTPUT));
    expect(prfBytes(new DataView(backing.buffer, 4, 32))).toEqual(Uint8Array.from(OUTPUT));
  });

  it("0〜255 の整数の配列も読む（1Password のブラウザー拡張が返す形）", () => {
    expect(prfBytes(OUTPUT)).toEqual(Uint8Array.from(OUTPUT));
  });

  it("元のバッファーを共有しない", () => {
    const source = Uint8Array.from(OUTPUT);
    const bytes = prfBytes(source)!;
    source[0] = 255;
    expect(bytes[0]).toBe(0);
  });

  it("32 バイトでないもの、範囲外の値、型の違うものは null", () => {
    expect(prfBytes(new Uint8Array(16))).toBeNull();
    expect(prfBytes(OUTPUT.slice(0, 31))).toBeNull();
    expect(prfBytes([...OUTPUT.slice(0, 31), 256])).toBeNull();
    expect(prfBytes([...OUTPUT.slice(0, 31), 1.5])).toBeNull();
    expect(prfBytes([...OUTPUT.slice(0, 31), "1"])).toBeNull();
    expect(prfBytes("AAAA")).toBeNull();
    // 欠けた要素のある配列（`Uint8Array.from` は 0 に変える）は読まない。
    const holes: number[] = [];
    holes.length = 32;
    expect(prfBytes(holes)).toBeNull();
    const sparse = OUTPUT.slice(0, 5);
    sparse[31] = 1;
    expect(prfBytes(sparse)).toBeNull();
    expect(prfBytes(undefined)).toBeNull();
    expect(prfBytes(null)).toBeNull();
  });
});
