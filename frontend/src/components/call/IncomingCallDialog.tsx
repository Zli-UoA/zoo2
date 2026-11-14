import { useLiveQuery } from "@tanstack/react-db";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { useCallInviteApi, useCallInvites } from "../../hooks/useCallInvites";
import { useCurrentUser } from "../../hooks/useCurrentUser";
import {
  acceptCallInvite,
  closeCallInviteNotification,
  consumeCallAcceptMark,
  declineCallInvite,
  parseCallInviteLaunch,
  stripCallInviteLaunch,
} from "../../lib/call/call-invites";
import { useCall } from "../../lib/call/call-context";
import { startRingtone } from "../../lib/call/call-sounds";
import { useOptionalCollections } from "../../lib/electric/collections-context";
import { isLocale } from "../../lib/i18n/locale";
import { formatDmDisplayName, resolveDmDisplayName } from "../chat/dm";
import { IncomingCallCard } from "./IncomingCallCard";

/** Service Worker から、開いているタブに送る「応答」「拒否」の依頼（`public/sw.js`）。 */
const SW_MESSAGE_TYPE = "zoo-call-invite";

/**
 * 着信のダイアログ。自分が呼ばれている招待があれば、応答・拒否を出して着信音を鳴らす。
 *
 * Web Push の「応答」は、Service Worker が残す一度きりの印（`consumeCallAcceptMark`）があるときだけ、
 * 応答して通話に入る。URL（`?call_invite=ID`）だけでは応答しない。
 */
export function IncomingCallDialog() {
  const { t, i18n } = useTranslation();
  const locale = isLocale(i18n.language) ? i18n.language : "ja";
  const { currentUserId } = useCurrentUser();
  const { incoming } = useCallInvites();
  const { joinCall } = useCall();
  const api = useCallInviteApi();
  const collections = useOptionalCollections();
  const users =
    useLiveQuery((q) => (collections ? q.from({ users: collections.users }) : undefined))?.data ??
    [];
  const channels =
    useLiveQuery((q) => (collections ? q.from({ channels: collections.channels }) : undefined))
      ?.data ?? [];
  const memberships =
    useLiveQuery((q) =>
      collections ? q.from({ channelUsers: collections.channelUsers }) : undefined,
    )?.data ?? [];
  // 処理中の招待。招待ごとに持つ（先に終わった処理が、別の招待の処理中を外さないように）。
  const [busyIds, setBusyIds] = useState<ReadonlySet<string>>(new Set());
  const [failedInviteId, setFailedInviteId] = useState<string | null>(null);
  const setBusy = useCallback((inviteId: string, busy: boolean) => {
    setBusyIds((current) => {
      const next = new Set(current);
      if (busy) next.add(inviteId);
      else next.delete(inviteId);
      return next;
    });
  }, []);

  const invite = incoming[0];
  const ringing = Boolean(invite);
  useEffect(() => {
    if (!ringing) return;
    return startRingtone();
  }, [ringing]);

  const accept = useCallback(
    async (inviteId: string) => {
      setBusy(inviteId, true);
      setFailedInviteId(null);
      try {
        const channelId = await acceptCallInvite(api, inviteId);
        // 終わっていた（期限切れ・取り消し）なら、入らない。
        if (!channelId) {
          toast.error(t("call.inviteEnded"));
          return;
        }
        // `joinCall` は、通話が終わるまで戻らない。待たずに、次の着信を操作できるようにする。
        void joinCall(channelId).catch(() => {});
      } catch {
        setFailedInviteId(inviteId);
      } finally {
        setBusy(inviteId, false);
        void closeCallInviteNotification(inviteId);
      }
    },
    [api, joinCall, setBusy, t],
  );

  const decline = useCallback(
    async (inviteId: string) => {
      setBusy(inviteId, true);
      try {
        await declineCallInvite(api, inviteId);
      } catch {
        // 拒否できなくても、期限で呼び出しは終わる。
      } finally {
        setBusy(inviteId, false);
        void closeCallInviteNotification(inviteId);
      }
    },
    [api, setBusy],
  );

  // Web Push の通知から開かれた。URL から外し、「応答」の印があったときだけ応答する。
  const launched = useRef(false);
  useEffect(() => {
    if (launched.current || !currentUserId) return;
    const inviteId = parseCallInviteLaunch(window.location.search);
    if (!inviteId) return;
    launched.current = true;
    window.history.replaceState(
      window.history.state,
      "",
      stripCallInviteLaunch(window.location.href),
    );
    void consumeCallAcceptMark(inviteId).then((marked) => {
      if (marked) void accept(inviteId);
    });
  }, [accept, currentUserId]);

  // Web Push の「応答」「拒否」（Service Worker が、開いているタブへ依頼する）。
  useEffect(() => {
    if (!("serviceWorker" in navigator)) return;
    const onMessage = (event: MessageEvent) => {
      const data = event.data as { type?: string; action?: string; inviteId?: string } | null;
      if (data?.type !== SW_MESSAGE_TYPE || typeof data.inviteId !== "string") return;
      const inviteId = data.inviteId;
      if (data.action === "decline") void decline(inviteId);
      else if (data.action === "accept") {
        void consumeCallAcceptMark(inviteId).then((marked) => {
          if (marked) void accept(inviteId);
        });
      }
    };
    navigator.serviceWorker.addEventListener("message", onMessage);
    return () => navigator.serviceWorker.removeEventListener("message", onMessage);
  }, [accept, decline]);

  const inviter = invite ? users.find((user) => user.id === invite.inviter_id) : undefined;
  const channel = invite ? channels.find((row) => row.id === invite.channel_id) : undefined;
  const inviterName = inviter?.display_name || inviter?.user_name || invite?.inviter_id || "";
  const channelLabel = useMemo(() => {
    if (!channel) return "";
    if (!channel.is_dm) return `#${channel.channel_name}`;
    return formatDmDisplayName(
      resolveDmDisplayName({
        channelName: channel.channel_name,
        members: memberships.filter((member) => member.channel_id === channel.id),
        users,
        currentUserId,
        locale,
      }),
      t,
    );
  }, [channel, currentUserId, locale, memberships, t, users]);

  if (!invite) return null;
  return (
    <IncomingCallCard
      inviterName={inviterName}
      inviterAvatarUrl={inviter?.avatar_url}
      channelLabel={channelLabel}
      isDm={Boolean(channel?.is_dm)}
      busy={busyIds.has(invite.id)}
      error={failedInviteId === invite.id ? t("call.inviteAcceptFailed") : null}
      onAccept={() => void accept(invite.id)}
      onDecline={() => void decline(invite.id)}
    />
  );
}
