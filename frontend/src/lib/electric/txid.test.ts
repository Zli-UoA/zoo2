import { describe, expect, it, vi } from "vite-plus/test";

import { changeMessageMatcher, syncTxid, type TxidSyncTarget } from "./txid";
import type { MessageRow } from "./types";

type ElectricMessage = Parameters<ReturnType<typeof changeMessageMatcher<MessageRow>>>[0];

function changeMessage(
  operation: "insert" | "update" | "delete",
  value: Partial<MessageRow>,
): ElectricMessage {
  return { key: value.id ?? "key", value, headers: { operation } } as unknown as ElectricMessage;
}

describe("changeMessageMatcher", () => {
  it("matches only the requested operation and row", () => {
    const match = changeMessageMatcher<MessageRow>("insert", (value) => value.id === "m1");

    expect(match(changeMessage("insert", { id: "m1" }))).toBe(true);
    expect(match(changeMessage("update", { id: "m1" }))).toBe(false);
    expect(match(changeMessage("insert", { id: "m2" }))).toBe(false);
  });

  it("excludes control messages such as up-to-date", () => {
    const match = changeMessageMatcher<MessageRow>("insert", () => true);

    expect(match({ headers: { control: "up-to-date" } } as unknown as ElectricMessage)).toBe(false);
  });
});

function makeTarget() {
  const awaitTxId = vi.fn<(txid: number, timeout?: number) => Promise<boolean>>(async () => true);
  const awaitMatch = vi.fn<
    (matchFn: (message: ElectricMessage) => boolean, timeout?: number) => Promise<boolean>
  >(async () => true);
  const target = { awaitTxId, awaitMatch } as unknown as TxidSyncTarget<MessageRow>;
  return { target, awaitTxId, awaitMatch };
}

describe("syncTxid", () => {
  it("resolves immediately in skip mode without touching the utilities", async () => {
    const { target, awaitTxId, awaitMatch } = makeTarget();

    await syncTxid({ mode: "skip", txid: "42", target, match: () => true });

    expect(awaitTxId).not.toHaveBeenCalled();
    expect(awaitMatch).not.toHaveBeenCalled();
  });

  it("does not await replicated changes for idempotent no-ops in either sync mode", async () => {
    const { target, awaitTxId, awaitMatch } = makeTarget();
    for (const mode of ["awaitTxId", "awaitMatch"] as const) {
      await syncTxid({ mode, txid: null, target, match: () => true });
    }
    expect(awaitTxId).not.toHaveBeenCalled();
    expect(awaitMatch).not.toHaveBeenCalled();
  });

  it("converts the string txid to a number for awaitTxId", async () => {
    const { target, awaitTxId, awaitMatch } = makeTarget();

    await syncTxid({ mode: "awaitTxId", txid: "42", target, match: () => true });

    expect(awaitTxId).toHaveBeenCalledWith(42, undefined);
    expect(awaitMatch).not.toHaveBeenCalled();
  });

  it("forwards the timeout to awaitTxId and awaitMatch", async () => {
    const { target, awaitTxId, awaitMatch } = makeTarget();

    await syncTxid({ mode: "awaitTxId", txid: "42", target, match: () => true, timeout: 5000 });
    await syncTxid({ mode: "awaitMatch", txid: "42", target, match: () => true, timeout: 6000 });

    expect(awaitTxId).toHaveBeenCalledWith(42, 5000);
    expect(awaitMatch).toHaveBeenCalledWith(expect.any(Function), 6000);
  });

  it("uses the match predicate in awaitMatch mode", async () => {
    const { target, awaitTxId, awaitMatch } = makeTarget();
    const match = changeMessageMatcher<MessageRow>("insert", (value) => value.id === "m1");

    await syncTxid({ mode: "awaitMatch", txid: "42", target, match });

    expect(awaitMatch).toHaveBeenCalledTimes(1);
    expect(awaitTxId).not.toHaveBeenCalled();
    const [passedMatch] = awaitMatch.mock.calls[0]!;
    expect(passedMatch(changeMessage("insert", { id: "m1" }))).toBe(true);
  });

  it("propagates the utility rejection so the caller can roll back", async () => {
    const { target, awaitTxId } = makeTarget();
    awaitTxId.mockRejectedValueOnce(new Error("timed out"));

    await expect(
      syncTxid({ mode: "awaitTxId", txid: "42", target, match: () => true }),
    ).rejects.toThrow("timed out");
  });
});
