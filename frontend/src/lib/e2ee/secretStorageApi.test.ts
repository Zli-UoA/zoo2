import type { KeySlot } from "@zoo/crypto-wasm";
import { expect, it } from "vite-plus/test";

import type { ApiClient } from "../api/client";
import { addKeySlot } from "./secretStorageApi";

it("sends the key id of the SSSS key that wrapped the slot, not the one the screen read", async () => {
  const bodies: Record<string, unknown>[] = [];
  const client = {
    post: async (_path: string, body: Record<string, unknown>) => {
      bodies.push(body);
      return { data: {}, txid: null };
    },
  } as unknown as ApiClient;
  const slot = {
    id: "slot",
    key_id: "old-key",
    kind: "recovery",
    nonce: "n",
    ciphertext: "c",
    label: null,
  } as KeySlot;
  // 別の端末が SSSS を作り直した（画面が読んだ既定の鍵は new-key）。サーバーが 409 で拒めるよう、
  // スロットを包んだ鍵の ID を送る。
  await addKeySlot(client, "new-key", slot, null);
  expect(bodies[0]).toMatchObject({ key_id: "old-key" });
  expect(bodies[0]).not.toHaveProperty("label");
});
