import { useMemo, useState, type ReactNode } from "react";

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

  const value = useMemo<ShellContextValue>(
    () => ({
      pane,
      showSidebar: () => setPane("sidebar"),
      showChat: () => setPane("chat"),
      membersOpen,
      setMembersOpen,
      toggleMembers: () => setMembersOpen((open) => !open),
      threadRootId,
      openThread: setThreadRootId,
      closeThread: () => setThreadRootId(null),
      selectedChannelId,
    }),
    [pane, membersOpen, threadRootId, selectedChannelId],
  );

  return <ShellContext.Provider value={value}>{children}</ShellContext.Provider>;
}
