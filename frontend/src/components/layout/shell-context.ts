import { createContext, useContext } from "react";

/**
 * 狭幅での表示面。`md` 未満ではサイドバーとチャットを切り替える。
 * `md` 以上は両方を並べて出すため、この値は効かない。
 */
export type ShellPane = "sidebar" | "chat";

export interface ShellContextValue {
  /** 狭幅でサイドバーとチャットのどちらを出すか。 */
  pane: ShellPane;
  showSidebar: () => void;
  showChat: () => void;
  /** メンバー（Members）パネル・画面の開閉。 */
  membersOpen: boolean;
  setMembersOpen: (open: boolean) => void;
  toggleMembers: () => void;
  /** ワイド画面で開いているスレッドのルートメッセージ id（未表示なら null）。 */
  threadRootId: string | null;
  /** スレッドを開く。メンバーパネルとは排他にする。 */
  openThread: (rootId: string) => void;
  closeThread: () => void;
  /** 現在選択中のチャンネル id（未選択なら null）。 */
  selectedChannelId: string | null;
}

export const ShellContext = createContext<ShellContextValue | null>(null);

export function useShell(): ShellContextValue {
  const context = useContext(ShellContext);
  if (!context) {
    throw new Error("useShell は AppShell の内側で使ってください。");
  }
  return context;
}
