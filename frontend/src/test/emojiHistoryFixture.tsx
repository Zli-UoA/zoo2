import { useMemo, type ReactNode } from "react";
import { EmojiHistoryContext, type EmojiHistory } from "../lib/emoji/history";
export const RECENT_EMOJIS_STORAGE_KEY = "zoo-test-recent-emoji";

export function EmojiHistoryFixture({ children }: { children: ReactNode }) {
  const history = useMemo<EmojiHistory>(() => {
    let values: string[] = [];
    const listeners = new Set<() => void>();
    const refresh = () => {
      const stored: string[] = JSON.parse(localStorage.getItem(RECENT_EMOJIS_STORAGE_KEY) ?? "[]");
      values = [...new Set(stored)];
      listeners.forEach((notify) => notify());
    };
    refresh();
    return {
      read: () => values,
      refresh,
      subscribe: (notify) => {
        listeners.add(notify);
        return () => {
          listeners.delete(notify);
        };
      },
      record: (emoji) => {
        localStorage.setItem(
          RECENT_EMOJIS_STORAGE_KEY,
          JSON.stringify([emoji, ...values.filter((item) => item !== emoji)].slice(0, 16)),
        );
        refresh();
      },
    };
  }, []);
  return <EmojiHistoryContext.Provider value={history}>{children}</EmojiHistoryContext.Provider>;
}
