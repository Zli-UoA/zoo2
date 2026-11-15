import type { JsonObject, SignatureUpload } from "@zoo/crypto-wasm";
import { expect, it } from "vite-plus/test";

import type { ApiClient } from "../api/client";
import { UPLOAD_BATCH, uploadPins, uploadSignaturesWithReauth } from "./revocationApi";

function recorder() {
  const bodies: { path: string; body: Record<string, unknown[]> }[] = [];
  const client = {
    post: async (path: string, body: Record<string, unknown[]>) => {
      bodies.push({ path, body });
      return { data: {}, txid: null };
    },
  } as unknown as ApiClient;
  return { client, bodies };
}

it("splits signatures into requests within the server limit", async () => {
  const { client, bodies } = recorder();
  const signatures = Array.from(
    { length: UPLOAD_BATCH + 1 },
    (_, i) => ({ target_user_id: `u${i}` }) as unknown as SignatureUpload,
  );
  await uploadSignaturesWithReauth(client, signatures, "token");
  expect(bodies.map((b) => b.body.signatures.length)).toEqual([UPLOAD_BATCH, 1]);
  await uploadSignaturesWithReauth(client, [], "token");
  expect(bodies).toHaveLength(2);
});

it("splits pins into requests within the server limit", async () => {
  const { client, bodies } = recorder();
  const pins = Array.from(
    { length: UPLOAD_BATCH * 2 + 1 },
    (_, i) => ({ target_user_id: `u${i}`, master_key: {}, signatures: {} }) as JsonObject,
  );
  await uploadPins(client, pins, true, "token");
  expect(bodies.map((b) => b.body.pins.length)).toEqual([UPLOAD_BATCH, UPLOAD_BATCH, 1]);
  expect(bodies.every((b) => (b.body as unknown as { replace: boolean }).replace)).toBe(true);
});
