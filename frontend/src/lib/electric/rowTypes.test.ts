import type { EncryptedMessage } from "@zoo/crypto-wasm";
import { describe, expect, expectTypeOf, it } from "vite-plus/test";

import { asElectricOptions, asPersistedOptions, asTableRow, asWasmCiphertext } from "./rowTypes";
import { coerceElectricRow } from "./schema";
import type { MegolmCiphertext, MessageRow, TableRowMap } from "./types";

describe("Electric 境界の型アダプタ", () => {
  it("生行をアプリの行型へ戻しても参照と値は変わらない", () => {
    const raw: Record<string, unknown> = { id: "message-1", revision: "2" };
    const row = asTableRow<"messages">(raw);

    // ライブラリ側の Record<string, unknown> に戻ってしまわないことを型チェックで固定する。
    expectTypeOf(row).toEqualTypeOf<MessageRow>();
    expectTypeOf(row).not.toBeAny();
    expect(row).toBe(raw);
    expect(row.revision).toBe("2");
  });

  it("値の正規化は coerceElectricRow が行い、アダプタは型だけを合わせる", () => {
    const row = coerceElectricRow("messages", {
      id: "message-1",
      created_at: "2026-11-27 00:00:00.000",
      revision: "2",
      apub: "false",
    });

    expectTypeOf(row).toEqualTypeOf<TableRowMap["messages"]>();
    expectTypeOf(row).not.toBeAny();
    expect(row.revision).toBe(2);
    expect(row.apub).toBe(false);
    expect(row.created_at).toBe("2026-11-27 00:00:00.000");
  });

  it("永続化オプションのアダプタは設定をそのまま渡す", () => {
    const transformer = (value: Record<string, unknown>) => value;
    const options = {
      id: "messages:c1",
      transformer,
      schemaVersion: 7,
      initialRender: { strategy: "network-first", networkTimeoutMs: 2_000 },
    };

    const persisted = asPersistedOptions(options);
    const electric = asElectricOptions<typeof options>(persisted);

    expectTypeOf(persisted).not.toBeAny();
    expectTypeOf(electric).toEqualTypeOf<typeof options>();
    expect(persisted).toBe(options);
    expect(electric).toBe(options);
    expect(electric.transformer).toBe(transformer);
    expect(electric.initialRender).toEqual({ strategy: "network-first", networkTimeoutMs: 2_000 });
  });

  it("暗号文のアダプタは値をそのまま wasm 側へ渡す", () => {
    const ciphertext = {
      algorithm: "m.megolm.v1.aes-sha2" as const,
      sender_key: "sender",
      device_id: "device",
      session_id: "session",
      ciphertext: "body",
    };

    const wasm = asWasmCiphertext(ciphertext);

    expectTypeOf(wasm).toEqualTypeOf<EncryptedMessage["ciphertext"]>();
    // null / undefined は暗号文ではない。引数型で弾けることを型チェックで固定する。
    expectTypeOf(asWasmCiphertext).parameter(0).toEqualTypeOf<MegolmCiphertext>();
    expect(wasm).toBe(ciphertext);
  });
});
