import { ShellSkeleton } from "./ShellSkeleton";
import { useCurrentUser } from "../../hooks/useCurrentUser";
import { OnboardingProvider } from "./OnboardingTour";
import "./navigation";
import { useRouter, useRouterState } from "@tanstack/react-router";
import { cn } from "cn";
import { useTranslation } from "react-i18next";
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";

import { isUuid } from "../../lib/api/path";
import { usePresence, useConversationView } from "../../hooks/usePresence";
import { AppSurfaceHost } from "../chat/AppSurfaceHost";
import { useActiveChannels } from "../../hooks/useRealtime";
import { useCall } from "../../lib/call/call-context";
import { MfmChannelsProvider } from "../chat/messages/MfmChannels";
import { MfmAccountsProvider } from "../chat/messages/MfmAccounts";
import { useAuthZAccounts } from "../../lib/emoji/accounts";
import { CallBar } from "../chat/CallBar";
import { DesktopBridge } from "../desktop/DesktopBridge";
import { AppBadgeBridge } from "../pwa/AppBadgeBridge";
import { ChannelSidebar } from "../chat/ChannelSidebar";
import { ChannelMembersDialog } from "../chat/ChannelMembersDialog";
import { SearchPreferencesProvider } from "../preferences/SearchPreferences";
import { PwaProvider } from "../pwa/PwaControls";
import { OfflineBanner } from "../offline/OfflineBanner";
import { AppUpdateNotice } from "../pwa/AppUpdateNotice";
import { LeftRail } from "./LeftRail";
import { ShellContext, type ShellContextValue, type ShellPane } from "./shell-context";
import { useMediaQuery } from "./useMediaQuery";
import { useVisualViewport } from "./useVisualViewport";

/**
 * アプリシェル。左レール＋サイドバー / メイン。
 *
 * レイアウトの表示は CSS のブレークポイントに任せ、JS は狭幅のペイン状態と
 * メンバーの開閉だけを持つ:
 *
 * - `md` (768px) 未満: `pane` に従ってサイドバーとチャットを切り替える。
 * - `md` 以上: サイドバーとチャットを並べる（`md:flex`）。
 * - チャンネルのメンバーは、画面幅によらず、スレッドのメンバーと同じ画面中央のダイアログに出す。
 *
 * メンバーの既定は閉じ。このコンポーネントは Electric のコレクションを触るため、
 * ルート側でマウント後の一度だけレンダリングする（SSR では呼ばない）。
 */
function AppShellContent({ children }: { children: ReactNode }) {
  const { t } = useTranslation();
  const accounts = useAuthZAccounts();
  usePresence();
  const viewportRef = useVisualViewport();
  const router = useRouter();
  const threadRootId = useRouterState({
    select: (state) => {
      const search = state.location.search as { thread?: unknown };
      return typeof search.thread === "string" ? search.thread : null;
    },
  });
  const isDesktop = useMediaQuery("(min-width: 768px)");

  const call = useCall();
  const isCallActive = Boolean(call.currentChannelId && call.connectionState !== "idle");

  const channelsQuery = useActiveChannels();
  const channels = channelsQuery.data ?? [];

  const isSettings = useRouterState({ select: (state) => state.location.pathname === "/settings" });
  const isHome = useRouterState({ select: (state) => state.location.pathname === "/" });
  const selectedChannelId = useRouterState({
    select: (state) => {
      for (let index = state.matches.length - 1; index >= 0; index -= 1) {
        const channelId = (state.matches[index].params as { channelId?: string }).channelId;
        // UUID でない値（細工した URL など）はチャンネルとして扱わず、API も呼ばない。
        if (isUuid(channelId)) {
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
  const [membersDialogOpen, setMembersDialogOpen] = useState(false);
  // 別のチャンネル・DM へ移ったら閉じる。前のチャンネルの一覧を、移った先で開いたままにしない。
  const [prevMembersChannelId, setPrevMembersChannelId] = useState(membersChannel?.id ?? null);
  if (prevMembersChannelId !== (membersChannel?.id ?? null)) {
    setPrevMembersChannelId(membersChannel?.id ?? null);
    setMembersDialogOpen(false);
  }
  const membersOpen = membersDialogOpen && !!membersChannel;

  const pane: ShellPane = isHome ? "sidebar" : "chat";
  const mainRef = useRef<HTMLElement>(null);
  const sidebarSkipRef = useRef<HTMLSpanElement>(null);

  const setMembersOpen = setMembersDialogOpen;

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
    },
    [router, selectedChannelId],
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

  const showSidebar = useCallback(() => {
    setMembersDialogOpen(false);
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
  // 狭幅のホームでは main が hidden なので、代わりに表示中のサイドバーへフォーカスする。
  const skipTarget = !showChat && !isDesktop ? sidebarSkipRef : mainRef;

  return (
    <SearchPreferencesProvider>
      <DesktopBridge channelId={selectedChannelId ?? null} />
      <AppBadgeBridge />
      <PwaProvider>
        <ShellContext.Provider value={value}>
          <MfmChannelsProvider channels={channels} ready={!channelsQuery.isInitialSyncLoading}>
            <MfmAccountsProvider accounts={accounts}>
              <div
                ref={viewportRef}
                data-app-shell
                className="fixed inset-x-0 top-0 flex h-dvh w-full overflow-hidden bg-background text-foreground"
              >
                <a
                  href="#main-content"
                  // フラグメント移動だと state が null の履歴が積まれ、thread/members の
                  // 閉じる処理が履歴を辿れなくなる。移動せずフォーカスだけを当てる。
                  onClick={(event) => {
                    event.preventDefault();
                    skipTarget.current?.focus();
                  }}
                  className="sr-only focus:not-sr-only focus:absolute focus:top-2 focus:left-2 focus:z-50 focus:rounded-md focus:bg-background focus:px-3 focus:py-2 focus:text-sm focus:font-medium focus:text-foreground focus:shadow-lg focus:ring-2 focus:ring-ring focus:outline-none"
                >
                  {t("a11y.skipToContent")}
                </a>
                <OfflineBanner />
                <AppUpdateNotice />
                <AppSurfaceHost />
                <div className={isSettings ? "hidden" : "contents"}>
                  <LeftRail />
                  {/* display:contents の親はフォーカスできないので、実体のある目標を置く。
                      共通ナビゲーションを飛ばすため LeftRail の後・サイドバーの前に置く。 */}
                  <span
                    ref={sidebarSkipRef}
                    id="sidebar-content"
                    tabIndex={-1}
                    className="sr-only"
                  />
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
                  ref={mainRef}
                  id="main-content"
                  tabIndex={-1}
                  className={cn(
                    "min-h-0 min-w-0 flex-1 flex-col",
                    showChat ? "flex" : "hidden md:flex",
                    isCallActive && "pb-20",
                  )}
                >
                  {children}
                </main>
                {membersChannel ? (
                  <ChannelMembersDialog
                    channel={membersChannel}
                    open={membersOpen}
                    onOpenChange={setMembersOpen}
                  />
                ) : null}
                <CallBar channels={channels} />
              </div>
            </MfmAccountsProvider>
          </MfmChannelsProvider>
        </ShellContext.Provider>
      </PwaProvider>
    </SearchPreferencesProvider>
  );
}

export function AppShell({ children }: { children: ReactNode }) {
  const router = useRouter();
  const channelId = useRouterState({
    select: (state) => {
      const id = state.location.pathname.startsWith("/channels/")
        ? state.location.pathname.split("/")[2]
        : null;
      return isUuid(id) ? id : null;
    },
  });
  const lastChannel = useRef<string | null>(null);
  useEffect(() => {
    if (channelId) lastChannel.current = channelId;
  }, [channelId]);
  const { currentUserId, isLoading } = useCurrentUser();
  if (isLoading && currentUserId === null) return <ShellSkeleton />;
  return (
    <OnboardingProvider
      key={currentUserId ?? "anonymous"}
      userId={currentUserId}
      channelSelected={!!channelId}
      onStepChange={async (step) => {
        if (step === "welcome" || step === "channels") await router.navigate({ to: "/" });
        if (step === "messages" && lastChannel.current)
          await router.navigate({
            to: "/channels/$channelId",
            params: { channelId: lastChannel.current },
          });
        if (step === "notifications") await router.navigate({ to: "/notifications" });
        if (step === "slack") await router.navigate({ to: "/settings", search: { tab: "slack" } });
        if (step === "settings")
          await router.navigate({ to: "/settings", search: { tab: "general" } });
      }}
    >
      <AppShellContent>{children}</AppShellContent>
    </OnboardingProvider>
  );
}
