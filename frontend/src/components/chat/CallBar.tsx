import { useLiveQuery } from "@tanstack/react-db";
import { Link } from "@tanstack/react-router";
import { Popover } from "@base-ui/react/popover";
import {
  ActivityIcon,
  HeadphonesIcon,
  MicIcon,
  MicOffIcon,
  PhoneOffIcon,
  Settings2Icon,
  UserPlusIcon,
  XIcon,
} from "lucide-react";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import { useCallInviteApi, useCallInvites } from "../../hooks/useCallInvites";
import { useCurrentUser } from "../../hooks/useCurrentUser";
import { useCall } from "../../lib/call/call-context";
import { cancelCallInvite, inviteToCall } from "../../lib/call/call-invites";
import { useOptionalCollections } from "../../lib/electric/collections-context";
import type { ChannelRow } from "../../lib/electric/types";
import { isLocale } from "../../lib/i18n/locale";
import { Button } from "../ui/button";
import { CallDeviceSelects } from "../call/CallDeviceSelects";
import { CallInvitePicker, type CallInviteCandidate } from "../call/CallInvitePicker";
import { CallStatusPanel } from "../call/CallStatusPanel";
import { formatDmDisplayName, resolveDmDisplayName } from "./dm";

import { AppAvatar } from "./AppIdentity";

interface CallBarProps {
  channels?: ChannelRow[];
}

export function CallBar({ channels = [] }: CallBarProps) {
  const { t, i18n } = useTranslation();
  const locale = isLocale(i18n.language) ? i18n.language : "ja";
  const { currentUserId } = useCurrentUser();
  const collections = useOptionalCollections();
  const users =
    useLiveQuery((q) => (collections ? q.from({ users: collections.users }) : undefined))?.data ??
    [];
  const memberships =
    useLiveQuery((q) =>
      collections ? q.from({ channelUsers: collections.channelUsers }) : undefined,
    )?.data ?? [];

  const {
    currentChannelId,
    connectionState,
    isMuted,
    participants,
    speakingUserIds,
    toggleMute,
    leaveCall,
    selectedMicId,
    selectedSpeakerId,
    setMicrophone,
    setSpeaker,
  } = useCall();

  const { outgoing } = useCallInvites();
  const inviteApi = useCallInviteApi();
  const [inviteBusy, setInviteBusy] = useState(false);
  const [inviteError, setInviteError] = useState<string | null>(null);

  const channel = channels.find((c) => c.id === currentChannelId);
  const channelOutgoing = outgoing.filter((invite) => invite.channel_id === currentChannelId);
  const userName = (id: string) => {
    const user = users.find((row) => row.id === id);
    return user?.display_name || user?.user_name || id;
  };

  // 呼べる人: チャンネルのメンバーで、通話に入っておらず、呼び出し中でもない人間のユーザー。
  const inviteCandidates = useMemo<CallInviteCandidate[]>(() => {
    if (!channel || channel.is_dm) return [];
    const busyIds = new Set([
      ...participants.map((p) => p.user_id),
      ...channelOutgoing.map((invite) => invite.invitee_id),
    ]);
    return memberships
      .filter((member) => member.channel_id === channel.id && member.user_id !== currentUserId)
      .map((member) => users.find((user) => user.id === member.user_id))
      .filter((user): user is NonNullable<typeof user> =>
        Boolean(user && !user.is_bot && !user.is_deactivated && !busyIds.has(user.id)),
      )
      .map((user) => ({
        id: user.id,
        name: user.display_name || user.user_name,
        avatarUrl: user.avatar_url,
      }));
  }, [channel, channelOutgoing, currentUserId, memberships, participants, users]);
  const inviteSlots = Math.max(0, 6 - participants.length - channelOutgoing.length);

  async function sendInvites(userIds: string[]) {
    if (!currentChannelId) return;
    setInviteBusy(true);
    setInviteError(null);
    try {
      await inviteToCall(inviteApi, currentChannelId, userIds);
    } catch {
      setInviteError(t("call.inviteFailed"));
    } finally {
      setInviteBusy(false);
    }
  }

  const channelName = useMemo(() => {
    if (!channel) return t("call.inCall");
    if (!channel.is_dm) return channel.channel_name;
    const members = memberships.filter((cu) => cu.channel_id === channel.id);
    const resolved = resolveDmDisplayName({
      channelName: channel.channel_name,
      members,
      users,
      currentUserId,
      locale,
    });
    return formatDmDisplayName(resolved, t);
  }, [channel, currentUserId, locale, memberships, t, users]);

  if (!currentChannelId || connectionState === "idle") {
    return null;
  }

  return (
    <aside
      aria-label={t("call.inCall")}
      className="fixed bottom-4 right-4 left-4 z-50 flex items-center justify-between gap-3 rounded-2xl border bg-card/95 px-4 py-2.5 shadow-xl backdrop-blur supports-backdrop-filter:bg-card/85 md:left-auto md:w-96"
    >
      <div className="flex min-w-0 items-center gap-3">
        <div className="relative flex size-3 shrink-0 items-center justify-center">
          {connectionState === "connected" ? (
            <>
              <span className="absolute inline-flex size-full animate-ping rounded-full bg-emerald-400 opacity-75" />
              <span className="relative inline-flex size-2.5 rounded-full bg-emerald-500" />
            </>
          ) : (
            <span className="relative inline-flex size-2.5 animate-pulse rounded-full bg-amber-500" />
          )}
        </div>

        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-1.5">
            <HeadphonesIcon
              className="size-3.5 shrink-0 text-muted-foreground"
              aria-hidden="true"
            />
            <Link
              to="/channels/$channelId"
              params={{ channelId: currentChannelId }}
              className="truncate text-xs font-semibold hover:underline"
            >
              {channelName}
            </Link>
          </div>

          {channelOutgoing.map((invite) => (
            <div
              key={invite.id}
              className="mt-0.5 flex items-center gap-1 text-[11px] text-muted-foreground"
            >
              <span className="min-w-0 truncate">
                {t("call.calling", { name: userName(invite.invitee_id) })}
              </span>
              <button
                type="button"
                className="-m-1 shrink-0 rounded-full p-1.5 hover:bg-muted"
                aria-label={t("call.cancelInvite", { name: userName(invite.invitee_id) })}
                title={t("call.cancelInvite", { name: userName(invite.invitee_id) })}
                onClick={() => void cancelCallInvite(inviteApi, invite.id).catch(() => {})}
              >
                <XIcon className="size-3" aria-hidden="true" />
              </button>
            </div>
          ))}

          <div className="mt-0.5 flex items-center gap-1">
            <span className="text-[11px] text-muted-foreground">
              {connectionState === "connecting"
                ? t("call.connecting")
                : t("call.participantCount", { count: participants.length })}
            </span>
            <div className="flex -space-x-1 overflow-hidden">
              {participants.slice(0, 4).map((p) => {
                const isSpeaking = speakingUserIds.has(p.user_id);
                const statusDetails: string[] = [];
                if (isSpeaking) statusDetails.push(t("call.speaking"));
                if (p.is_muted) statusDetails.push(t("call.muted"));
                const label =
                  statusDetails.length > 0
                    ? `${p.display_name} (${statusDetails.join(", ")})`
                    : p.display_name;

                return (
                  <div
                    key={p.session_id}
                    role="img"
                    aria-label={label}
                    title={label}
                    className={`relative flex size-5 shrink-0 items-center justify-center rounded-full bg-muted text-[10px] font-medium transition-all ${
                      isSpeaking ? "ring-2 ring-emerald-500" : ""
                    }`}
                  >
                    <AppAvatar
                      label={p.display_name}
                      src={users.find((user) => user.id === p.user_id)?.avatar_url}
                      className="size-5 text-[10px]"
                    />
                    {p.is_muted ? (
                      <span
                        aria-hidden="true"
                        className="absolute -bottom-0.5 -right-0.5 rounded-full bg-destructive p-0.5 text-destructive-foreground"
                      >
                        <MicOffIcon className="size-2" />
                      </span>
                    ) : null}
                  </div>
                );
              })}
              {participants.length > 4 ? (
                <div className="flex size-5 shrink-0 items-center justify-center rounded-full bg-muted text-[9px] text-muted-foreground">
                  +{participants.length - 4}
                </div>
              ) : null}
            </div>
          </div>
        </div>
      </div>

      <div className="flex shrink-0 items-center gap-1.5">
        {channel && !channel.is_dm ? (
          <Popover.Root>
            <Popover.Trigger
              render={
                <Button
                  type="button"
                  variant="secondary"
                  size="icon-sm"
                  className="size-9 rounded-full"
                  aria-label={t("call.inviteButton")}
                  title={t("call.inviteButton")}
                  disabled={connectionState !== "connected"}
                />
              }
            >
              <UserPlusIcon className="size-4" />
            </Popover.Trigger>
            <Popover.Portal>
              <Popover.Positioner side="top" align="end" sideOffset={8} className="z-[60]">
                <Popover.Popup className="w-[min(20rem,calc(100vw-2rem))] rounded-xl border bg-popover p-4 text-popover-foreground shadow-xl outline-none">
                  <Popover.Title className="mb-3 text-sm font-semibold">
                    {t("call.inviteButton")}
                  </Popover.Title>
                  <CallInvitePicker
                    candidates={inviteCandidates}
                    slots={inviteSlots}
                    busy={inviteBusy}
                    error={inviteError}
                    onInvite={(ids) => void sendInvites(ids)}
                  />
                </Popover.Popup>
              </Popover.Positioner>
            </Popover.Portal>
          </Popover.Root>
        ) : null}

        <Popover.Root>
          <Popover.Trigger
            render={
              <Button
                type="button"
                variant="secondary"
                size="icon-sm"
                className="size-9 rounded-full"
                aria-label={t("call.statusButton")}
                title={t("call.statusButton")}
              />
            }
          >
            <ActivityIcon className="size-4" />
          </Popover.Trigger>
          <Popover.Portal>
            <Popover.Positioner side="top" align="end" sideOffset={8} className="z-[60]">
              <Popover.Popup className="max-h-[70vh] w-[min(22rem,calc(100vw-2rem))] overflow-y-auto rounded-xl border bg-popover p-4 text-popover-foreground shadow-xl outline-none">
                <Popover.Title className="mb-3 text-sm font-semibold">
                  {t("call.statusButton")}
                </Popover.Title>
                <CallStatusPanel />
              </Popover.Popup>
            </Popover.Positioner>
          </Popover.Portal>
        </Popover.Root>

        <Popover.Root>
          <Popover.Trigger
            render={
              <Button
                type="button"
                variant="secondary"
                size="icon-sm"
                className="size-9 rounded-full"
                aria-label={t("call.devicesButton")}
                title={t("call.devicesButton")}
              />
            }
          >
            <Settings2Icon className="size-4" />
          </Popover.Trigger>
          <Popover.Portal>
            <Popover.Positioner side="top" align="end" sideOffset={8} className="z-[60]">
              <Popover.Popup className="w-[min(20rem,calc(100vw-2rem))] rounded-xl border bg-popover p-4 text-popover-foreground shadow-xl outline-none">
                <Popover.Title className="mb-3 text-sm font-semibold">
                  {t("call.devicesButton")}
                </Popover.Title>
                <CallDeviceSelects
                  micId={selectedMicId}
                  speakerId={selectedSpeakerId}
                  onMicChange={(id) => void setMicrophone(id)}
                  onSpeakerChange={(id) => void setSpeaker(id)}
                />
              </Popover.Popup>
            </Popover.Positioner>
          </Popover.Portal>
        </Popover.Root>

        <Button
          type="button"
          variant={isMuted ? "destructive" : "secondary"}
          size="icon-sm"
          className="size-9 rounded-full"
          aria-label={isMuted ? t("call.unmute") : t("call.mute")}
          title={isMuted ? t("call.unmute") : t("call.mute")}
          onClick={() => void toggleMute()}
        >
          {isMuted ? <MicOffIcon className="size-4" /> : <MicIcon className="size-4" />}
        </Button>

        <Button
          type="button"
          variant="destructive"
          size="icon-sm"
          className="size-9 rounded-full"
          aria-label={t("call.leave")}
          title={t("call.leave")}
          onClick={() => void leaveCall()}
        >
          <PhoneOffIcon className="size-4" />
        </Button>
      </div>
    </aside>
  );
}
