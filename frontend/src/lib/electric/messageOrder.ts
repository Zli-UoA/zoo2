/**
 * メッセージ一覧の並び (作成日時・IDの新しい順) と、それを on-demand の subset snapshot で取るためのインデックス。
 *
 * `created_at` は `YYYY-MM-DD HH:MM:SS.sss` の文字列で持つ。TanStack DB は、文字列の列で並べて `limit` を付けた
 * クエリを、`stringSort: "lexical"` (サーバーの並びと一致する) のときだけ、ページ単位 (`subset__limit` と
 * `subset__order_by`) で取れる。それ以外 (既定の `locale`) では、全行を取るフォールバックになる。
 * クエリの `orderBy` と、`messages` コレクションのインデックスは、同じ比較の設定にそろえる。
 */
export const MESSAGE_ORDER = { direction: "desc", stringSort: "lexical" } as const;

/** `created_at` のインデックスの比較の設定 (方向はインデックスと問い合わせで異なってよい)。 */
export const MESSAGE_INDEX_COMPARE_OPTIONS = {
  direction: "asc",
  nulls: "first",
  stringSort: "lexical",
} as const;

/** 上へスクロールするときに、1 回で読み足す件数 (トップレベルのメッセージ、またはスレッドの返信)。 */
export const MESSAGE_PAGE_SIZE = 50;
