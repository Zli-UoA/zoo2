import { act, renderHook, waitFor } from "@testing-library/react";
import { expect, it, vi } from "vite-plus/test";

import type { FeedMessage } from "../components/chat/messages/messageFormat";
import type { CryptoWorker } from "../lib/e2ee/cryptoWorker";
import type { DecryptOutcome } from "../lib/e2ee/store/megolm";
import { useMessageDecryptions } from "./useMessageDecryptions";

const e2ee = vi.hoisted(() => ({ worker: null as unknown, roomKeysVersion: 0 }));
vi.mock("../lib/e2ee/e2ee-context", () => ({ useE2ee: () => e2ee }));

const message = {
  id: "m1",
  channel_id: "c1",
  user_id: "alice",
  parent_id: null,
  message_kind: "message",
  epoch: "1",
  revision: 0,
  content: "",
  ciphertext: { session_id: "s", ciphertext: "c" },
} as unknown as FeedMessage;

const decrypted = {
  status: "decrypted",
  body: "hello",
  kind: "message",
  sender: "verified",
  revision: { status: "latest" },
} as unknown as DecryptOutcome;
const keyMissing = { status: "undecryptable", reason: "keyMissing" } as DecryptOutcome;

function worker(request: () => Promise<DecryptOutcome[]>) {
  return { request: vi.fn(request) } as unknown as CryptoWorker & {
    request: ReturnType<typeof vi.fn>;
  };
}

it("ignores a late response from the previous worker after the worker changes", async () => {
  let resolveOld: (outcomes: DecryptOutcome[]) => void = () => {};
  const oldWorker = worker(
    () =>
      new Promise((resolve) => {
        resolveOld = resolve;
      }),
  );
  const newWorker = worker(async () => [decrypted]);

  e2ee.worker = oldWorker;
  const { result, rerender } = renderHook(({ messages }) => useMessageDecryptions("c1", messages), {
    initialProps: { messages: [message] },
  });
  await waitFor(() => expect(oldWorker.request).toHaveBeenCalledTimes(1));

  // ロックやアカウントの切り替えで Worker が変わった。新しい Worker で復号できた。
  e2ee.worker = newWorker;
  rerender({ messages: [message] });
  await waitFor(() => expect(result.current.get("m1")?.status).toBe("decrypted"));

  // 古い Worker の応答が遅れて届いても、キャッシュに入れない。
  await act(async () => resolveOld([keyMissing]));
  rerender({ messages: [{ ...message }] });
  await waitFor(() => expect(result.current.get("m1")).toMatchObject({ status: "decrypted" }));
  expect(newWorker.request).toHaveBeenCalledTimes(1);
});
