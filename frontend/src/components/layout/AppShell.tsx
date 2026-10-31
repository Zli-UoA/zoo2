import "./navigation";
import { useRouter, useRouterState } from "@tanstack/react-router";
import { cn } from "cn";
import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";

import { usePresence, useConversationView } from "../../hooks/usePresence";
import { AppSurfaceHost } from "../chat/AppSurfaceHost";
import { useActiveChannels } from "../../hooks/useRealtime";
import { MfmChannelsProvider } from "../chat/messages/MfmChannels";
import { ChannelSidebar } from "../chat/ChannelSidebar";
import { MembersDialog } from "../chat/MembersDialog";
import { MembersPanel } from "../chat/MembersPanel";
import { InstallPromptProvider } from "../pwa/PwaControls";
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
  usePresence();
  const viewportRef = useVisualViewport();
  const router = useRouter();
  const [membersOpen, setMembersOpen] = useState(false);
  const threadRootId = useRouterState({
    select: (state) => {
      const search = state.location.search as { thread?: unknown };
      return typeof search.thread === "string" ? search.thread : null;
    },
  });
  const isWide = useMediaQuery("(min-width: 1024px)");

  const channelsQuery = useActiveChannels();
  const channels = channelsQuery.data ?? [];

  const isHome = useRouterState({ select: (state) => state.location.pathname === "/" });
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
  useConversationView(selectedChannelId);
  const selectedChannel = useMemo(
    () => channels.find((channel) => channel.id === selectedChannelId) ?? null,
    [channels, selectedChannelId],
  );
  const membersChannel = selectedChannel && !selectedChannel.is_dm ? selectedChannel : null;

  const [prevSelectedChannelIsDm, setPrevSelectedChannelIsDm] = useState(false);
  const selectedChannelIsDm = selectedChannel?.is_dm ?? false;
  if (prevSelectedChannelIsDm !== selectedChannelIsDm) {
    setPrevSelectedChannelIsDm(selectedChannelIsDm);
    if (selectedChannelIsDm) {
      setMembersOpen(false);
    }
  }

  const pane: ShellPane = isHome ? "sidebar" : "chat";

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

  // メンバーとスレッドは右ペインを奪い合うため、ワイドでは排他にする。
  // スレッドを開いたときに閉じる処理は `openThread` が行い、メンバーを開いた
  // ときもスレッドを閉じる。
  // 狭幅はスレッドがオーバーレイ、メンバーがダイアログなので重ねたままにする。

  const openThread = useCallback(
    (rootId: string) => {
      if (!selectedChannelId) return;
      const current = router.state.location;
      const search = current.search as { message?: string; thread?: string };
      if (search.thread !== rootId) {
        void router.navigate({
          to: "/channels/$channelId",
          params: { channelId: selectedChannelId },
          search: { ...search, thread: rootId },
          replace: !!search.thread,
          state: (previous) => ({
            ...previous,
            zooThreadParent: search.thread ? previous.zooThreadParent : selectedChannelId,
          }),
        });
      }
      if (isWide) setMembersOpen(false);
    },
    [isWide, router, selectedChannelId],
  );
  const closeThread = useCallback(() => {
    if (!selectedChannelId) return;
    const current = router.state.location;
    if (!(current.search as { thread?: string }).thread) return;
    if (current.state.zooThreadParent === selectedChannelId && router.history.canGoBack()) {
      router.history.back();
    } else {
      void router.navigate({
        to: "/channels/$channelId",
        params: { channelId: selectedChannelId },
        search: {},
        replace: true,
      });
    }
  }, [router, selectedChannelId]);

  useEffect(() => {
    if (isWide && membersOpen && threadRootId) closeThread();
  }, [isWide, membersOpen, threadRootId, closeThread]);

  const showSidebar = useCallback(() => {
    setMembersOpen(false);
    if ((router.state.location.state.__TSR_index ?? 0) > 0) router.history.back();
    else void router.navigate({ to: "/", replace: true });
  }, [router]);

  const value = useMemo<ShellContextValue>(
    () => ({
      pane,
      showSidebar,
      showChat: () => {},
      membersOpen,
      setMembersOpen,
      toggleMembers: () => setMembersOpen((open) => !open),
      threadRootId,
      openThread,
      closeThread,
      selectedChannelId,
    }),
    [pane, showSidebar, membersOpen, threadRootId, openThread, closeThread, selectedChannelId],
  );

  const sidebarVisible = pane === "sidebar";
  const showChat = pane === "chat";

  return (
    <InstallPromptProvider>
      <ShellContext.Provider value={value}>
        <MfmChannelsProvider channels={channels}>
          <div
            ref={viewportRef}
            data-app-shell
            className="fixed inset-x-0 top-0 flex h-dvh w-full overflow-hidden bg-background text-foreground"
          >
            <OfflineBanner />
            <AppSurfaceHost />
            <LeftRail />
            <ChannelSidebar
              channels={channels}
              loading={channelsQuery.isLoading}
              error={channelsQuery.isError}
              className={sidebarVisible ? "flex" : "hidden md:flex"}
            />
            <main className={cn("min-w-0 flex-1 flex-col", showChat ? "flex" : "hidden md:flex")}>
              {children}
            </main>
            {/* 表示は CSS（`hidden lg:flex`）が決める。狭幅でダイアログと二重に
            マウントしないよう、マウントだけ `isWide` で抑える。 */}
            {isWide && membersOpen && membersChannel ? (
              <MembersPanel
                channel={membersChannel}
                className="hidden w-72 shrink-0 border-l lg:flex xl:w-80"
              />
            ) : null}
            {membersChannel ? (
              <MembersDialog
                channel={membersChannel}
                open={membersOpen && !isWide}
                onOpenChange={setMembersOpen}
              />
            ) : null}
          </div>
        </MfmChannelsProvider>
      </ShellContext.Provider>
    </InstallPromptProvider>
  );
}
