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

describe("unreadableMembers", () => {
  it("マスター鍵と自己署名鍵があり、端末もある人は読める", () => {
    const result = unreadableMembers([
      page({
        user_ids: ["alice"],
        master_keys: { alice: { keys: {} } },
        self_signing_keys: { alice: { keys: {} } },
        device_keys: { alice: { D1: { device_id: "D1" } } },
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

  it("設定はしたが端末がない人は、使える端末がない", () => {
    const result = unreadableMembers([
      page({
        user_ids: ["dave"],
        master_keys: { dave: { keys: {} } },
        self_signing_keys: { dave: { keys: {} } },
        device_keys: { dave: {} },
      }),
    ]);
    expect(result.get("dave")).toBe("noDevices");
  });

  it("複数のページをまとめて見る", () => {
    const result = unreadableMembers([
      page({ user_ids: ["a"] }),
      page({
        user_ids: ["b"],
        master_keys: { b: {} },
        self_signing_keys: { b: {} },
        device_keys: { b: { D: {} } },
      }),
    ]);
    expect([...result.entries()]).toEqual([["a", "notSetUp"]]);
  });
});
