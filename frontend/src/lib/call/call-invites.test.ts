import { afterEach, describe, expect, it, vi } from "vite-plus/test";

import { ApiError } from "../api/client";
import type { CallInviteRow } from "../electric/types";
import {
  CALL_ACCEPT_MARK_TTL_MS,
  acceptCallInvite,
  callAcceptMarkPath,
  callInviteDurationMs,
  callInviteExpiresAt,
  consumeCallAcceptMark,
  consumeCallMark,
  cancelCallInvite,
  declineCallInvite,
  inviteToCall,
  liveCallInvites,
  msUntilNextExpiry,
  parseCallInviteLaunch,
  splitCallInvites,
  stripCallInviteLaunch,
} from "./call-invites";

const NOW = Date.parse("2026-11-14T10:00:10Z");

function invite(patch: Partial<CallInviteRow> = {}): CallInviteRow {
  return {
    id: "00000000-0000-4000-8000-000000000001",
    channel_id: "c1",
    inviter_id: "alice",
    invitee_id: "bob",
    status: "pending",
    created_at: "2026-11-14 10:00:00.000+00",
    expires_at: "2026-11-14 10:00:30.000+00",
    ...patch,
  };
}

describe("callInviteExpiresAt", () => {
  it("Electric の timestamptz（+00）を読む", () => {
    expect(callInviteExpiresAt(invite())).toBe(Date.parse("2026-11-14T10:00:30Z"));
  });
  it("T 区切りや Z、+09:00 も読む", () => {
    expect(callInviteExpiresAt(invite({ expires_at: "2026-11-14T10:00:30Z" }))).toBe(
      Date.parse("2026-11-14T10:00:30Z"),
    );
    expect(callInviteExpiresAt(invite({ expires_at: "2026-11-14 19:00:30+09:00" }))).toBe(
      Date.parse("2026-11-14T10:00:30Z"),
    );
  });
  it("読めなければ期限切れ（0）", () => {
    expect(callInviteExpiresAt(invite({ expires_at: "nonsense" }))).toBe(0);
  });
});

describe("liveCallInvites / splitCallInvites", () => {
  it("pending で、期限前のものだけを、期限の早い順に返す", () => {
    const rows = [
      invite({ id: "b", expires_at: "2026-11-14 10:00:40+00" }),
      invite({ id: "a", expires_at: "2026-11-14 10:00:20+00" }),
      invite({ id: "gone", status: "declined" }),
      invite({ id: "late", expires_at: "2026-11-14 10:00:05+00" }),
    ];
    expect(liveCallInvites(rows, NOW).map((row) => row.id)).toEqual(["a", "b"]);
  });

  it("自分が呼ばれたものと、自分が呼んでいるものに分ける", () => {
    const rows = [
      invite({ id: "in", invitee_id: "me", inviter_id: "alice" }),
      invite({ id: "out", invitee_id: "bob", inviter_id: "me" }),
      invite({ id: "other", invitee_id: "bob", inviter_id: "alice" }),
    ];
    const { incoming, outgoing } = splitCallInvites(rows, "me", NOW);
    expect(incoming.map((row) => row.id)).toEqual(["in"]);
    expect(outgoing.map((row) => row.id)).toEqual(["out"]);
  });

  it("ユーザーが未確定なら、どちらも空", () => {
    expect(splitCallInvites([invite()], null, NOW)).toEqual({ incoming: [], outgoing: [] });
  });
});

describe("msUntilNextExpiry", () => {
  it("次に期限が来るまでの時間を返し、なければ null", () => {
    expect(msUntilNextExpiry([invite()], NOW)).toBe(20_000);
    expect(msUntilNextExpiry([], NOW)).toBeNull();
    expect(msUntilNextExpiry([invite({ status: "missed" })], NOW)).toBeNull();
  });
});

describe("API", () => {
  it("応答は channel_id を返し、終わっていれば null", async () => {
    const request = vi.fn().mockResolvedValueOnce({ ok: true, channel_id: "c1" });
    expect(await acceptCallInvite({ request }, "i1")).toBe("c1");
    expect(request).toHaveBeenCalledWith("/api/call-invites/i1/accept", { method: "POST" });
    const gone = vi.fn().mockRejectedValue(new ApiError(409, "gone"));
    expect(await acceptCallInvite({ request: gone }, "i1")).toBeNull();
  });

  it("応答の失敗が 409・404 以外なら投げる", async () => {
    const request = vi.fn().mockRejectedValue(new ApiError(500, "boom"));
    await expect(acceptCallInvite({ request }, "i1")).rejects.toThrow("boom");
  });

  it("拒否と取り消しは、すでに終わっていても成功と同じ", async () => {
    const gone = vi.fn().mockRejectedValue(new ApiError(404, "gone"));
    await expect(declineCallInvite({ request: gone }, "i1")).resolves.toBeUndefined();
    await expect(cancelCallInvite({ request: gone }, "i1")).resolves.toBeUndefined();
    const request = vi.fn().mockResolvedValue({ ok: true });
    await cancelCallInvite({ request }, "i1");
    expect(request).toHaveBeenCalledWith("/api/call-invites/i1/cancel", { method: "POST" });
  });

  it("招待は user_ids を送る", async () => {
    const request = vi.fn().mockResolvedValue({ invites: [] });
    await inviteToCall({ request }, "c1", ["u1", "u2"]);
    expect(request).toHaveBeenCalledWith("/api/channels/c1/call/invites", {
      method: "POST",
      body: { user_ids: ["u1", "u2"] },
    });
  });
});

describe("起動用の URL パラメーター", () => {
  const id = "00000000-0000-4000-8000-000000000001";
  it("招待の ID だけを読む（応答の指定は読まない）", () => {
    expect(parseCallInviteLaunch(`?call_invite=${id}`)).toBe(id);
    expect(parseCallInviteLaunch(`?call_invite=${id}&call_action=accept`)).toBe(id);
  });
  it("UUID でなければ読まない", () => {
    expect(parseCallInviteLaunch("?call_invite=../../x")).toBeNull();
    expect(parseCallInviteLaunch("")).toBeNull();
  });
  it("URL から外しても、他のパラメーターは残る", () => {
    expect(
      stripCallInviteLaunch(`https://zoo.example/channels/c1?message=m1&call_invite=${id}#x`),
    ).toBe("/channels/c1?message=m1#x");
  });
});

describe("応答の印（Service Worker が残す）", () => {
  const id = "00000000-0000-4000-8000-000000000001";

  function stubCaches(entries: Map<string, string>) {
    vi.stubGlobal("caches", {
      open: async () => ({
        match: async (path: string) => {
          // 本物の Response と同じく、あとで印が消えても本文は読める。
          const value = entries.get(path);
          return value === undefined ? undefined : { text: async () => value };
        },
        delete: async (path: string) => entries.delete(path),
      }),
    });
  }

  afterEach(() => vi.unstubAllGlobals());

  it("印があれば一度だけ true を返し、印は消える", async () => {
    const entries = new Map([[callAcceptMarkPath(id), String(1_000)]]);
    stubCaches(entries);
    expect(await consumeCallAcceptMark(id, 1_500)).toBe(true);
    expect(await consumeCallAcceptMark(id, 1_500)).toBe(false);
  });

  it("拒否の印は、応答の印とは別に扱う", async () => {
    const entries = new Map([["/__call-decline/" + id, String(1_000)]]);
    stubCaches(entries);
    expect(await consumeCallMark("accept", id, 1_500)).toBe(false);
    expect(await consumeCallMark("decline", id, 1_500)).toBe(true);
    expect(await consumeCallMark("decline", id, 1_500)).toBe(false);
  });

  it("印がなければ（URL を開いただけでは）false", async () => {
    stubCaches(new Map());
    expect(await consumeCallAcceptMark(id)).toBe(false);
  });

  it("古い印は使えない（消えるが false）", async () => {
    const entries = new Map([[callAcceptMarkPath(id), String(1_000)]]);
    stubCaches(entries);
    expect(await consumeCallAcceptMark(id, 1_000 + CALL_ACCEPT_MARK_TTL_MS + 1)).toBe(false);
    expect(entries.size).toBe(0);
  });

  it("Cache が使えなければ false", async () => {
    vi.stubGlobal("caches", {
      open: async () => {
        throw new Error("blocked");
      },
    });
    expect(await consumeCallAcceptMark(id)).toBe(false);
  });
});

describe("callInviteDurationMs と端末の時計", () => {
  it("招待の長さは created_at と expires_at の差", () => {
    expect(callInviteDurationMs(invite())).toBe(30_000);
    expect(callInviteDurationMs(invite({ created_at: "nonsense" }))).toBe(0);
  });

  it("deadlineOf を渡すと、端末の時計がずれていても期限を決められる", () => {
    const skewedNow = NOW + 120_000; // 端末の時計が 2 分進んでいる
    const row = invite();
    expect(liveCallInvites([row], skewedNow)).toEqual([]);
    const receivedAt = skewedNow - 1_000;
    const deadlineOf = () => receivedAt + callInviteDurationMs(row);
    expect(liveCallInvites([row], skewedNow, deadlineOf)).toEqual([row]);
  });
});
