import { describe, expect, it } from "vite-plus/test";
import type { JsonObject } from "@zoo/crypto-wasm";

import { unreadableMembers } from "./memberReadability";

function page(overrides: Partial<Record<string, unknown>> = {}): JsonObject {
  return {
    channel_id: "c1",
    membership_epoch: "3",
    user_ids: [],
    device_keys: {},
    master_keys: {},
    self_signing_keys: {},
    user_signing_keys: {},
    signatures: [],
    tofu_pins: {},
    next_after: null,
    ...overrides,
  } as JsonObject;
}

const SSK = "ed25519:SSKPUB";
const crossSigning = (user: string) => ({
  master_keys: { [user]: { keys: { "ed25519:MSKPUB": "MSKPUB" } } },
  self_signing_keys: { [user]: { keys: { [SSK]: "SSKPUB" } } },
});

describe("unreadableMembers", () => {
  it("自己署名鍵で署名された端末（端末鍵の signatures）がある人は読める", () => {
    const result = unreadableMembers([
      page({
        user_ids: ["alice"],
        ...crossSigning("alice"),
        device_keys: {
          alice: { D1: { device_id: "D1", signatures: { alice: { [SSK]: "sig" } } } },
        },
      }),
    ]);
    expect(result.size).toBe(0);
  });

  it("自己署名鍵の署名が signatures の行にある端末も数える", () => {
    const result = unreadableMembers([
      page({
        user_ids: ["alice"],
        ...crossSigning("alice"),
        device_keys: { alice: { D1: { device_id: "D1" } } },
        signatures: [
          {
            signer_user_id: "alice",
            signer_key_id: SSK,
            target_user_id: "alice",
            target_key_id: "D1",
            signature: "sig",
          },
        ],
      }),
    ]);
    expect(result.size).toBe(0);
  });

  it("マスター鍵か自己署名鍵がない人は、暗号化を設定していない", () => {
    const result = unreadableMembers([
      page({
        user_ids: ["bob", "carol"],
        master_keys: { carol: { keys: {} } },
        self_signing_keys: {},
        device_keys: { bob: { D1: {} }, carol: { D2: {} } },
      }),
    ]);
    expect(result.get("bob")).toBe("notSetUp");
    expect(result.get("carol")).toBe("notSetUp");
  });

  it("端末がない人は、使える端末がない", () => {
    const result = unreadableMembers([
      page({ user_ids: ["dave"], ...crossSigning("dave"), device_keys: { dave: {} } }),
    ]);
    expect(result.get("dave")).toBe("noDevices");
  });

  it("署名済みの端末を消し、署名のない端末だけが残った人も、使える端末がない", () => {
    const result = unreadableMembers([
      page({
        user_ids: ["erin"],
        ...crossSigning("erin"),
        device_keys: {
          erin: { D2: { device_id: "D2", signatures: { erin: { "ed25519:D2": "self" } } } },
        },
        // 別の鍵（古い自己署名鍵）の行や、別の端末への行は数えない。
        signatures: [
          {
            signer_user_id: "erin",
            signer_key_id: "ed25519:OLDSSK",
            target_user_id: "erin",
            target_key_id: "D2",
            signature: "old",
          },
          {
            signer_user_id: "erin",
            signer_key_id: SSK,
            target_user_id: "erin",
            target_key_id: "D1",
            signature: "deleted",
          },
        ],
      }),
    ]);
    expect(result.get("erin")).toBe("noDevices");
  });

  it("複数のページをまとめて見る", () => {
    const result = unreadableMembers([
      page({ user_ids: ["a"] }),
      page({
        user_ids: ["b"],
        ...crossSigning("b"),
        device_keys: { b: { D: { signatures: { b: { [SSK]: "sig" } } } } },
      }),
    ]);
    expect([...result.entries()]).toEqual([["a", "notSetUp"]]);
  });
});
