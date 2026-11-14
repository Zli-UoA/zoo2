import { useLiveQuery } from "@tanstack/react-db";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { useCallInviteApi, useCallInvites } from "../../hooks/useCallInvites";
import { useCurrentUser } from "../../hooks/useCurrentUser";
import {
  acceptCallInvite,
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

/** Service Worker から、開いているタブに送る「拒否」の依頼（`public/sw.js`）。 */
const SW_MESSAGE_TYPE = "zoo-call-invite";

/**
 * 着信のダイアログ。自分が呼ばれている招待があれば、応答・拒否を出して着信音を鳴らす。
 * Web Push の「応答」で開かれたとき（`?call_invite=ID&call_action=accept`）は、そのまま応答する。
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
  const [busy, setBusy] = useState(false);
  const [failedInviteId, setFailedInviteId] = useState<string | null>(null);

  const invite = incoming[0];
  const ringing = Boolean(invite);
  useEffect(() => {
    if (!ringing) return;
    return startRingtone();
  }, [ringing]);

  const accept = useCallback(
    async (inviteId: string) => {
      setBusy(true);
      try {
        const channelId = await acceptCallInvite(api, inviteId);
        // 終わっていた（期限切れ・取り消し）なら、入らない。
        if (!channelId) {
          setFailedInviteId(inviteId);
          return;
        }
        await joinCall(channelId);
      } catch {
        setFailedInviteId(inviteId);
      } finally {
        setBusy(false);
      }
    },
    [api, joinCall],
  );

  const decline = useCallback(
    async (inviteId: string) => {
      setBusy(true);
      try {
        await declineCallInvite(api, inviteId);
      } catch {
        // 拒否できなくても、期限で呼び出しは終わる。
      } finally {
        setBusy(false);
      }
    },
    [api],
  );

  // Web Push の「応答」で開かれた。URL から外してから、一度だけ応答する。
  const launched = useRef(false);
  useEffect(() => {
    if (launched.current || !currentUserId) return;
    const launch = parseCallInviteLaunch(window.location.search);
    if (!launch) return;
    launched.current = true;
    window.history.replaceState(
      window.history.state,
      "",
      stripCallInviteLaunch(window.location.href),
    );
    if (launch.action === "accept") void accept(launch.inviteId);
  }, [accept, currentUserId]);

  // Web Push の「拒否」（開いているタブへ Service Worker が依頼する）。
  useEffect(() => {
    if (!("serviceWorker" in navigator)) return;
    const onMessage = (event: MessageEvent) => {
      const data = event.data as { type?: string; action?: string; inviteId?: string } | null;
      if (data?.type !== SW_MESSAGE_TYPE || typeof data.inviteId !== "string") return;
      if (data.action === "decline") void decline(data.inviteId);
    };
    navigator.serviceWorker.addEventListener("message", onMessage);
    return () => navigator.serviceWorker.removeEventListener("message", onMessage);
  }, [decline]);

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
      busy={busy}
      error={failedInviteId === invite.id ? t("call.inviteEnded") : null}
      onAccept={() => void accept(invite.id)}
      onDecline={() => void decline(invite.id)}
    />
  );
}
