// @vitest-environment happy-dom
import { act, renderHook, waitFor } from "@testing-library/react";
import { createElement, type ReactNode } from "react";
import { afterEach, expect, it, vi } from "vite-plus/test";

import { resolveConfig } from "../config";
import { useMessageFeed } from "../../hooks/useRealtime";
import { getCollections } from "./collections";
import { CollectionsProvider } from "./collections-context";
import { MESSAGE_PAGE_SIZE } from "./messageOrder";

afterEach(() => vi.unstubAllGlobals());

const TOTAL = MESSAGE_PAGE_SIZE * 3 + 7;
const channelId = "ondemand-channel";
const pad = (n: number) => String(n).padStart(4, "0");
/** 古い順に msg-0001 ... (created_at も同じ順)。 */
const rows = Array.from({ length: TOTAL }, (_, index) => {
  const n = index + 1;
  return {
    id: `msg-${pad(n)}`,
    user_id: "u1",
    channel_id: channelId,
    content: `message ${n}`,
    created_at: `2026-01-01 00:${String(Math.floor(n / 60)).padStart(2, "0")}:${String(n % 60).padStart(2, "0")}.000`,
    updated_at: null,
    deleted_at: null,
    parent_id: null as string | null,
    display_data: '{"emojis":{},"attachments":[],"reactions":[],"reply_count":0}',
    message_kind: "message",
    broadcast: "false",
    apub: "false",
  };
});

/** Electric の subset snapshot を、必要な範囲だけで再現する (`parent_id IS NULL` と `created_at < $1`)。 */
function subsetPage(params: URLSearchParams) {
  const values = JSON.parse(params.get("subset__params") ?? "{}") as Record<string, string>;
  const where = params.get("subset__where") ?? "true = true";
  let out = rows.filter((row) => row.deleted_at === null);
  if (/"parent_id"\s+IS\s+NULL/i.test(where)) out = out.filter((row) => row.parent_id === null);
  const bound = /"created_at"\s*(<=|=|<)\s*\$(\d+)/.exec(where);
  if (bound) {
    const value = values[bound[2]!]!;
    out = out.filter((row) =>
      bound[1] === "<"
        ? row.created_at < value
        : bound[1] === "<="
          ? row.created_at <= value
          : row.created_at === value,
    );
  }
  out = [...out].sort((a, b) => (a.created_at < b.created_at ? 1 : -1));
  return out.slice(0, Number(params.get("subset__limit") ?? out.length));
}

it("loads only one page per request and widens the range page by page", async () => {
  const requests: URLSearchParams[] = [];
  vi.stubGlobal(
    "fetch",
    vi.fn<typeof fetch>((input, init) => {
      const url = new URL(typeof input === "string" ? input : (input as Request).url);
      const hang = () =>
        new Promise<Response>((_resolve, reject) => {
          init?.signal?.addEventListener("abort", () => reject(init.signal?.reason), {
            once: true,
          });
        });
      const live = url.searchParams.get("live") === "true" || url.searchParams.has("live_sse");
      if (!url.pathname.endsWith("/messages")) {
        // users など、ほかの Shape は、空の同期済みの応答を返す (live の購読は取得中のまま)。
        if (live) return hang();
        return Promise.resolve(
          new Response(JSON.stringify([{ headers: { control: "up-to-date" } }]), {
            headers: {
              "content-type": "application/json",
              "electric-handle": `h-${url.pathname}`,
              "electric-offset": "0_0",
              "electric-schema": JSON.stringify({ id: { type: "text" } }),
            },
          }),
        );
      }
      requests.push(url.searchParams);
      if (!url.searchParams.has("subset__where") && !url.searchParams.has("subset__limit")) {
        // ストリーム (変更の購読)。応答せず、取得中のままにする。
        return hang();
      }
      const data = subsetPage(url.searchParams).map((row) => ({
        key: row.id,
        value: Object.fromEntries(
          Object.entries(row).map(([key, value]) => [key, value === null ? null : String(value)]),
        ),
        headers: { operation: "insert", relation: ["public", "messages"] },
      }));
      return Promise.resolve(
        new Response(
          JSON.stringify({
            metadata: { xmin: "1", xmax: "1", xip_list: [], snapshot_mark: 1, database_lsn: "1" },
            data,
          }),
          {
            headers: {
              "content-type": "application/json",
              "electric-handle": "h-messages",
              "electric-offset": "0_0",
              "electric-schema": JSON.stringify({ id: { type: "text" } }),
            },
          },
        ),
      );
    }),
  );
  const bundle = getCollections(
    resolveConfig({ DEV: true, VITE_SHAPE_MODE: "proxy", VITE_DEV_USER_ID: "u1" }),
    { namespace: "ondemand-feed-test", persistence: null },
  );
  const { result, unmount } = renderHook(() => useMessageFeed(channelId), {
    wrapper: ({ children }: { children: ReactNode }) =>
      createElement(CollectionsProvider, { value: bundle, children }),
  });
  try {
    await waitFor(() => expect(result.current.data?.length).toBe(MESSAGE_PAGE_SIZE), {
      timeout: 3_000,
    });
    // 古い順に並べる。最新の 1 ページだけで、先頭は「新しい方から 50 件目」。
    expect(result.current.data?.at(-1)?.id).toBe(`msg-${pad(TOTAL)}`);
    expect(result.current.data?.[0]?.id).toBe(`msg-${pad(TOTAL - MESSAGE_PAGE_SIZE + 1)}`);
    expect(result.current.hasOlder).toBe(true);

    await act(async () => {
      await result.current.loadOlder();
    });
    await waitFor(() => expect(result.current.data?.length).toBe(MESSAGE_PAGE_SIZE * 2), {
      timeout: 3_000,
    });
    expect(result.current.data?.[0]?.id).toBe(`msg-${pad(TOTAL - MESSAGE_PAGE_SIZE * 2 + 1)}`);

    // 全件を取るフォールバック (ページの要求ではない取得) になっていない。
    // `stringSort: "lexical"` と、同じ設定のインデックスが外れると、ここで失敗する。
    // 取得したのは、ページの要求 (limit と order_by つき) と、境界の確認 (同じ時刻の行を数える。数行だけ) だけ。
    const snapshots = requests.filter((params) => params.has("subset__where"));
    expect(snapshots.length).toBeGreaterThan(0);
    const pages = snapshots.filter((params) => params.has("subset__limit"));
    expect(pages.length).toBeGreaterThanOrEqual(2);
    for (const params of pages) {
      expect(params.get("subset__order_by")).toMatch(/created_at.*DESC/i);
      expect(Number(params.get("subset__limit"))).toBeLessThanOrEqual(MESSAGE_PAGE_SIZE + 1);
    }
    for (const params of snapshots.filter((value) => !value.has("subset__limit"))) {
      expect(params.get("subset__where")).toMatch(/"created_at"\s*(=|<=)\s*\$1/);
    }
    expect(bundle.messages(channelId).size).toBeLessThanOrEqual(MESSAGE_PAGE_SIZE * 2 + 1);
  } finally {
    unmount();
    await Promise.all([
      bundle.messages(channelId).cleanup(),
      bundle.users.cleanup(),
      bundle.reactions(channelId).cleanup(),
    ]);
  }
});
