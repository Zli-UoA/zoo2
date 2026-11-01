import "./navigation";
import { useRouter, useRouterState } from "@tanstack/react-router";
import { cn } from "cn";
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";

import { usePresence, useConversationView } from "../../hooks/usePresence";
import { AppSurfaceHost } from "../chat/AppSurfaceHost";
import { useActiveChannels } from "../../hooks/useRealtime";
import { MfmChannelsProvider } from "../chat/messages/MfmChannels";
import { ChannelSidebar } from "../chat/ChannelSidebar";
import { MembersPanel } from "../chat/MembersPanel";
import { SearchPreferencesProvider } from "../preferences/SearchPreferences";
import { PwaProvider } from "../pwa/PwaControls";
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
 * - `lg` (1024px) 以上: メンバーを右ペインへ出す。狭幅は同じ一覧を
 *   履歴に対応した全画面で表示する。
 *
 * メンバーの既定は閉じ。このコンポーネントは Electric のコレクションを触るため、
 * ルート側でマウント後の一度だけレンダリングする（SSR では呼ばない）。
 */
export function AppShell({ children }: { children: ReactNode }) {
  usePresence();
  const viewportRef = useVisualViewport();
  const router = useRouter();
  const [membersPanelOpen, setMembersPanelOpen] = useState(false);
  const membersTriggerRef = useRef<HTMLElement | null>(null);
  const membersRequested = useRouterState({
    select: (state) => (state.location.search as { members?: boolean }).members === true,
  });
  const threadRootId = useRouterState({
    select: (state) => {
      const search = state.location.search as { thread?: unknown };
      return typeof search.thread === "string" ? search.thread : null;
    },
  });
  const isWide = useMediaQuery("(min-width: 1024px)");

  const channelsQuery = useActiveChannels();
  const channels = channelsQuery.data ?? [];

  const isSettings = useRouterState({ select: (state) => state.location.pathname === "/settings" });
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
  const membersOpen = isWide ? membersPanelOpen : membersRequested;

  const [prevSelectedChannelIsDm, setPrevSelectedChannelIsDm] = useState(false);
  const selectedChannelIsDm = selectedChannel?.is_dm ?? false;
  if (prevSelectedChannelIsDm !== selectedChannelIsDm) {
    setPrevSelectedChannelIsDm(selectedChannelIsDm);
    if (selectedChannelIsDm) {
      setMembersPanelOpen(false);
    }
  }

  const pane: ShellPane = isHome ? "sidebar" : "chat";

  const setMembersOpen = useCallback(
    (open: boolean) => {
      if (isWide) {
        setMembersPanelOpen(open);
        return;
      }
      if (!selectedChannelId) return;
      const current = router.state.location;
      const search = current.search as { message?: string; thread?: string; members?: boolean };
      if (!!search.members === open) return;
      if (open) {
        membersTriggerRef.current =
          document.activeElement instanceof HTMLElement ? document.activeElement : null;
        void router.navigate({
          to: "/channels/$channelId",
          params: { channelId: selectedChannelId },
          search: { ...search, members: true },
          state: (previous) => ({ ...previous, zooMembersParent: selectedChannelId }),
        });
      } else if (
        current.state.zooMembersParent === selectedChannelId &&
        router.history.canGoBack()
      ) {
        router.history.back();
      } else {
        void router.navigate({
          to: "/channels/$channelId",
          params: { channelId: selectedChannelId },
          search: { ...search, members: undefined },
          replace: true,
        });
      }
    },
    [isWide, router, selectedChannelId],
  );

  // Changing the layout closes the members view, as with the desktop side panel.
  const [prevIsWide, setPrevIsWide] = useState(isWide);
  if (prevIsWide !== isWide) {
    setPrevIsWide(isWide);
    setMembersPanelOpen(false);
  }
  useEffect(() => {
    if (isWide && membersRequested && selectedChannelId) {
      const search = router.state.location.search as { message?: string; thread?: string };
      void router.navigate({
        to: "/channels/$channelId",
        params: { channelId: selectedChannelId },
        search: { ...search, members: undefined },
        replace: true,
      });
    }
    if (!membersRequested && membersTriggerRef.current) {
      if (membersTriggerRef.current.isConnected) membersTriggerRef.current.focus();
      membersTriggerRef.current = null;
    }
  }, [isWide, membersRequested, router, selectedChannelId]);

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
    [isWide, router, selectedChannelId, setMembersOpen],
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

  const previousThreadRoot = useRef(threadRootId);
  useEffect(() => {
    const enteredThread = previousThreadRoot.current !== threadRootId;
    previousThreadRoot.current = threadRootId;
    if (isWide && membersOpen && threadRootId) {
      // History navigation into a thread takes precedence over the open members panel.
      if (enteredThread) setMembersOpen(false);
      else closeThread();
    }
  }, [isWide, membersOpen, threadRootId, closeThread, setMembersOpen]);

  const showSidebar = useCallback(() => {
    setMembersPanelOpen(false);
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
      toggleMembers: () => setMembersOpen(!membersOpen),
      threadRootId,
      openThread,
      closeThread,
      selectedChannelId,
    }),
    [
      pane,
      showSidebar,
      membersOpen,
      setMembersOpen,
      threadRootId,
      openThread,
      closeThread,
      selectedChannelId,
    ],
  );

  // The conversation column belongs to the channel index and channel/DM routes only.
  const isChannelView = isHome || selectedChannelId !== null;
  const sidebarVisible = pane === "sidebar";
  const showChat = pane === "chat";
  const showMembersScreen = !isWide && membersOpen && !!membersChannel;

  return (
    <SearchPreferencesProvider>
      <PwaProvider>
        <ShellContext.Provider value={value}>
          <MfmChannelsProvider channels={channels}>
            <div
              ref={viewportRef}
              data-app-shell
              className="fixed inset-x-0 top-0 flex h-dvh w-full overflow-hidden bg-background text-foreground"
            >
              <OfflineBanner />
              <AppSurfaceHost />
              <div className={isSettings || showMembersScreen ? "hidden" : "contents"}>
                <LeftRail />
                <ChannelSidebar
                  channels={channels}
                  loading={channelsQuery.isLoading}
                  error={channelsQuery.isError}
                  className={
                    isChannelView ? (sidebarVisible ? "flex" : "hidden md:flex") : "hidden"
                  }
                />
              </div>
              <main
                className={cn(
                  "min-h-0 min-w-0 flex-1 flex-col",
                  showChat ? "flex" : "hidden md:flex",
                )}
              >
                <div className={showMembersScreen ? "hidden" : "contents"}>{children}</div>
                {showMembersScreen && membersChannel ? (
                  <MembersPanel
                    channel={membersChannel}
                    className="flex h-full"
                    onBack={() => setMembersOpen(false)}
                  />
                ) : null}
              </main>
              {/* 一覧は現在の配置先だけにマウントする。 */}
              {isWide && membersOpen && membersChannel ? (
                <MembersPanel
                  channel={membersChannel}
                  className="hidden w-72 shrink-0 border-l lg:flex xl:w-80"
                />
              ) : null}
            </div>
          </MfmChannelsProvider>
        </ShellContext.Provider>
      </PwaProvider>
    </SearchPreferencesProvider>
  );
}
