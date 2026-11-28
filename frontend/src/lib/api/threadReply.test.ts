import { describe, expect, it, vi } from "vite-plus/test";

import { ApiError, type ApiClient, type ApiEnvelope } from "./client";
import { isThreadBusyError, postThreadReply } from "./threadReply";

function busyError(retryAfter?: number, bodyRetryAfter?: number): ApiError {
  const body: Record<string, unknown> = {
    error: "The thread is busy; retry shortly",
    code: "THREAD_BUSY",
  };
  if (bodyRetryAfter !== undefined) body.retry_after = bodyRetryAfter;
  return new ApiError(
    503,
    "Request to /api/messages failed with status 503",
    body,
    { path: "/api/messages", method: "POST" },
    retryAfter ?? null,
  );
}

function clientReturning(results: unknown[]): {
  client: ApiClient;
  post: ReturnType<typeof vi.fn>;
} {
  const post = vi.fn(async (_path: string, _body?: unknown, _options?: unknown) => {
    const result = results.shift();
    if (result instanceof Error) throw result;
    return result as ApiEnvelope<unknown>;
  });
  return { client: { post } as unknown as ApiClient, post };
}

const ok: ApiEnvelope<{ id: string }> = { data: { id: "m1" }, txid: "100" };

describe("isThreadBusyError", () => {
  it("only matches a 503 that carries the THREAD_BUSY code", () => {
    expect(isThreadBusyError(busyError())).toBe(true);
    expect(isThreadBusyError(new ApiError(503, "busy", { code: "OTHER" }))).toBe(false);
    expect(isThreadBusyError(new ApiError(409, "busy", { code: "THREAD_BUSY" }))).toBe(false);
    expect(isThreadBusyError(new Error("busy"))).toBe(false);
  });
});

describe("postThreadReply", () => {
  it("resends the identical body after Retry-After seconds and returns the success", async () => {
    const delays: number[] = [];
    const { client, post } = clientReturning([busyError(5), ok]);
    const body = { id: "m1", content: "hello", parent_id: "root" };

    await expect(
      postThreadReply(client, body, {
        delay: async (ms) => {
          delays.push(ms);
        },
      }),
    ).resolves.toEqual(ok);

    expect(post).toHaveBeenCalledTimes(2);
    expect(post.mock.calls[0]).toEqual(["/api/messages", body]);
    expect(post.mock.calls[1]).toEqual(["/api/messages", body]);
    expect(post.mock.calls[1]?.[1]).toBe(body);
    expect(delays).toEqual([5000]);
  });

  it("falls back to the body's retry_after when the header was not exposed by CORS", async () => {
    const delays: number[] = [];
    const { client } = clientReturning([busyError(undefined, 7), ok]);
    await postThreadReply(
      client,
      { id: "m1" },
      {
        delay: async (ms) => {
          delays.push(ms);
        },
      },
    );
    expect(delays).toEqual([7000]);
  });

  it("gives up after the retry limit and rethrows the last error", async () => {
    const delays: number[] = [];
    const last = busyError(5);
    const { client, post } = clientReturning([busyError(5), busyError(5), last]);

    await expect(
      postThreadReply(
        client,
        { id: "m1" },
        {
          delay: async (ms) => {
            delays.push(ms);
          },
        },
      ),
    ).rejects.toBe(last);

    // 最初の試行 + 上限 2 回 = 3 回まで。それ以上は送らない。
    expect(post).toHaveBeenCalledTimes(3);
    expect(delays).toEqual([5000, 5000]);
  });

  it("does not retry other failures", async () => {
    const delay = vi.fn(async () => undefined);
    for (const error of [
      new ApiError(503, "busy", { code: "OTHER" }),
      new ApiError(409, "conflict", { code: "THREAD_BUSY" }),
      new Error("network"),
    ]) {
      const { client, post } = clientReturning([error]);
      await expect(postThreadReply(client, { id: "m1" }, { delay })).rejects.toBe(error);
      expect(post).toHaveBeenCalledTimes(1);
    }
    expect(delay).not.toHaveBeenCalled();
  });

  it("forwards the abort signal to every attempt", async () => {
    const signal = new AbortController().signal;
    const { client, post } = clientReturning([busyError(0), ok]);
    await postThreadReply(client, { id: "m1" }, { signal, delay: async () => undefined });
    expect(post.mock.calls[0]?.[2]).toEqual({ signal });
    expect(post.mock.calls[1]?.[2]).toEqual({ signal });
  });
});
