import { useRouterState } from "@tanstack/react-router";
import { cn } from "cn";
import { useEffect, useMemo, useState, type ReactNode } from "react";

import { useActiveChannels } from "../../hooks/useRealtime";
import { ChannelSidebar } from "../chat/ChannelSidebar";
import { MembersDialog } from "../chat/MembersDialog";
import { MembersPanel } from "../chat/MembersPanel";
import { LeftRail } from "./LeftRail";
import { ShellContext, type ShellContextValue, type ShellPane } from "./shell-context";
import { useMediaQuery } from "./useMediaQuery";

/**
 * 3ペインのアプリシェル。左レール＋サイドバー / メイン / 右メンバー。
 *
 * レイアウトの表示は CSS のブレークポイントに任せ、JS は狭幅のペイン状態と
 * メンバーの開閉・マウントだけを持つ:
 *
 * - `md` (768px) 未満: `pane` に従ってサイドバーとチャットを切り替える。
 * - `md` 以上: サイドバーとチャットを並べる（`md:flex`）。
 * - `lg` (1024px) 以上: メンバーペインを `lg:flex` で出す。狭幅ではペインを
 *   マウントせず、ポータルで出る `MembersDialog` へ切り替える（ポータルは CSS で
 *   切り替えられないため `isWide` を見る）。
 *
 * メンバーの既定は閉じ。このコンポーネントは Electric のコレクションを触るため、
 * ルート側でマウント後の一度だけレンダリングする（SSR では呼ばない）。
 */
export function AppShell({ children }: { children: ReactNode }) {
  const [pane, setPane] = useState<ShellPane>("sidebar");
  const [membersOpen, setMembersOpen] = useState(false);
  const isWide = useMediaQuery("(min-width: 1024px)");

  const channelsQuery = useActiveChannels();
  const channels = channelsQuery.data ?? [];

  const selectedChannelId = useRouterState({
    select: (state) => {
      for (let index = state.matches.length - 1; index >= 0; index -= 1) {
        const channelId = (state.matches[index].params as { channelId?: string }).channelId;
        if (channelId) {
          return channelId;
        }
      }
      return null;
    },
  });
  const selectedChannel = useMemo(
    () => channels.find((channel) => channel.id === selectedChannelId) ?? null,
    [channels, selectedChannelId],
  );

  // チャンネルを開いたら狭幅ではチャット面へ、未選択ならサイドバーへ戻す。
  useEffect(() => {
    setPane(selectedChannelId ? "chat" : "sidebar");
  }, [selectedChannelId]);

  // ワイドから狭幅へ縮んだときに、ペインで開いていたメンバーが
  // そのままダイアログとして出ないように閉じる。コミット前に反映するため
  // レンダー中に調整する（effect だとダイアログが一度マウントされて残る）。
  const [prevIsWide, setPrevIsWide] = useState(isWide);
  if (prevIsWide !== isWide) {
    setPrevIsWide(isWide);
    if (!isWide) {
      setMembersOpen(false);
    }
  }

  const value = useMemo<ShellContextValue>(
    () => ({
      pane,
      showSidebar: () => setPane("sidebar"),
      showChat: () => setPane("chat"),
      membersOpen,
      setMembersOpen,
      toggleMembers: () => setMembersOpen((open) => !open),
      selectedChannelId,
    }),
    [pane, membersOpen, selectedChannelId],
  );

  const showSidebar = pane === "sidebar";
  const showChat = pane === "chat";

  return (
    <ShellContext.Provider value={value}>
      <div className="flex h-dvh w-full overflow-hidden bg-background text-foreground">
        <LeftRail />
        <ChannelSidebar
          channels={channels}
          loading={channelsQuery.isLoading}
          error={channelsQuery.isError}
          className={showSidebar ? "flex" : "hidden md:flex"}
        />
        <main className={cn("min-w-0 flex-1 flex-col", showChat ? "flex" : "hidden md:flex")}>
          {children}
        </main>
        {/* 表示は CSS（`hidden lg:flex`）が決める。狭幅でダイアログと二重に
            マウントしないよう、マウントだけ `isWide` で抑える。 */}
        {isWide && membersOpen && selectedChannel ? (
          <MembersPanel
            channel={selectedChannel}
            className="hidden w-72 shrink-0 border-l lg:flex xl:w-80"
          />
        ) : null}
        {selectedChannel ? (
          <MembersDialog
            channel={selectedChannel}
            open={membersOpen && !isWide}
            onOpenChange={setMembersOpen}
          />
        ) : null}
      </div>
    </ShellContext.Provider>
  );
}
