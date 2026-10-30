import { useCallback, useMemo, useState, type ReactNode } from "react";

import {
  ShellContext,
  type ShellContextValue,
  type ShellPane,
} from "../src/components/layout/shell-context";

export function StoryShell({
  children,
  selectedChannelId = "channel-general",
  initialPane = "chat",
  initialMembersOpen = false,
  initialThreadRootId = null,
}: {
  children: ReactNode;
  selectedChannelId?: string | null;
  initialPane?: ShellPane;
  initialMembersOpen?: boolean;
  initialThreadRootId?: string | null;
}) {
  const [pane, setPane] = useState<ShellPane>(initialPane);
  const [membersOpen, setMembersOpen] = useState(initialMembersOpen);
  const [threadRootId, setThreadRootId] = useState<string | null>(initialThreadRootId);

  const showSidebar = useCallback(() => setPane("sidebar"), []);
  const showChat = useCallback(() => setPane("chat"), []);
  const toggleMembers = useCallback(() => setMembersOpen((open) => !open), []);
  const openThread = useCallback((rootId: string) => setThreadRootId(rootId), []);
  const closeThread = useCallback(() => setThreadRootId(null), []);

  const value = useMemo<ShellContextValue>(
    () => ({
      pane,
      showSidebar,
      showChat,
      membersOpen,
      setMembersOpen,
      toggleMembers,
      threadRootId,
      openThread,
      closeThread,
      selectedChannelId,
    }),
    [
      pane,
      showSidebar,
      showChat,
      membersOpen,
      toggleMembers,
      threadRootId,
      openThread,
      closeThread,
      selectedChannelId,
    ],
  );

  return <ShellContext.Provider value={value}>{children}</ShellContext.Provider>;
}
