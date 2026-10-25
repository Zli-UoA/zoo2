import { useRouterState } from "@tanstack/react-router";
import { cn } from "cn";
import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";

import { useActiveChannels } from "../../hooks/useRealtime";
import { MfmChannelsProvider } from "../chat/messages/MfmChannels";
import { ChannelSidebar } from "../chat/ChannelSidebar";
import { MembersDialog } from "../chat/MembersDialog";
import { MembersPanel } from "../chat/MembersPanel";
import { OfflineBanner } from "../offline/OfflineBanner";
import { LeftRail } from "./LeftRail";
import { ShellContext, type ShellContextValue, type ShellPane } from "./shell-context";
import { useMediaQuery } from "./useMediaQuery";
import { useVisualViewport } from "./useVisualViewport";

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
  const viewportRef = useVisualViewport();
  const [pane, setPane] = useState<ShellPane>("sidebar");
  const [membersOpen, setMembersOpen] = useState(false);
  const [threadRootId, setThreadRootId] = useState<string | null>(null);
  const isWide = useMediaQuery("(min-width: 1024px)");

  const channelsQuery = useActiveChannels();
  const channels = channelsQuery.data ?? [];

  const isNotifications = useRouterState({
    select: (state) => state.location.pathname === "/notifications",
  });
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

  // チャンネル・通知を開いたら狭幅でもメイン画面へ、ホームではサイドバーへ戻す。
  useEffect(() => {
    setPane(selectedChannelId || isNotifications ? "chat" : "sidebar");
  }, [selectedChannelId, isNotifications]);

  // ワイドから狭幅へ縮んだときに、ペインで開いていたメンバーが
  // そのままダイアログとして出ないように閉じる。コミット前に反映するため
  // レンダー中に調整する（effect だとダイアログが一度マウントされて残る）。
  const [prevIsWide, setPrevIsWide] = useState(isWide);
  if (prevIsWide !== isWide) {
    setPrevIsWide(isWide);
    if (!isWide) {
      setMembersOpen(false);
    } else if (membersOpen) {
      // 狭幅でダイアログとスレッドが重なったまま広げた場合は、右ペインを
      // 奪い合わないようスレッドを閉じる。
      setThreadRootId(null);
    }
  }

  // メンバーとスレッドは右ペインを奪い合うため、ワイドでは排他にする。
  // スレッドを開いたときに閉じる処理は `openThread` が行い、メンバーを開いた
  // ときはここでスレッドを閉じる（レンダー中に反映して一瞬の同時表示を避ける）。
  // 狭幅はスレッドがオーバーレイ、メンバーがダイアログなので重ねたままにする。
  const [prevMembersOpen, setPrevMembersOpen] = useState(membersOpen);
  if (prevMembersOpen !== membersOpen) {
    setPrevMembersOpen(membersOpen);
    if (membersOpen && isWide) {
      setThreadRootId(null);
    }
  }

  const openThread = useCallback(
    (rootId: string) => {
      setThreadRootId(rootId);
      if (isWide) {
        setMembersOpen(false);
      }
    },
    [isWide],
  );
  const closeThread = useCallback(() => setThreadRootId(null), []);

  const value = useMemo<ShellContextValue>(
    () => ({
      pane,
      showSidebar: () => setPane("sidebar"),
      showChat: () => setPane("chat"),
      membersOpen,
      setMembersOpen,
      toggleMembers: () => setMembersOpen((open) => !open),
      threadRootId,
      openThread,
      closeThread,
      selectedChannelId,
    }),
    [pane, membersOpen, threadRootId, openThread, closeThread, selectedChannelId],
  );

  const showSidebar = !isNotifications && pane === "sidebar";
  const showChat = isNotifications || pane === "chat";

  return (
    <ShellContext.Provider value={value}>
      <MfmChannelsProvider channels={channels}>
      <div
        ref={viewportRef}
        data-app-shell
        className="fixed inset-x-0 top-0 flex h-dvh w-full overflow-hidden bg-background text-foreground"
      >
        <OfflineBanner />
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
      </MfmChannelsProvider>
    </ShellContext.Provider>
  );
}
