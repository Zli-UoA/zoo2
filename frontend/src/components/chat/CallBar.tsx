import { Link } from "@tanstack/react-router";
import { HeadphonesIcon, MicIcon, MicOffIcon, PhoneOffIcon } from "lucide-react";
import { useTranslation } from "react-i18next";

import { useCall } from "../../lib/call/call-context";
import type { ChannelRow } from "../../lib/electric/types";
import { Button } from "../ui/button";

interface CallBarProps {
  channels?: ChannelRow[];
}

export function CallBar({ channels = [] }: CallBarProps) {
  const { t } = useTranslation();
  const {
    currentChannelId,
    connectionState,
    isMuted,
    participants,
    speakingUserIds,
    toggleMute,
    leaveCall,
  } = useCall();

  if (!currentChannelId || connectionState === "idle") {
    return null;
  }

  const channel = channels.find((c) => c.id === currentChannelId);
  const channelName = channel ? channel.channel_name : t("call.inCall");

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

          <div className="mt-0.5 flex items-center gap-1">
            <span className="text-[11px] text-muted-foreground">
              {connectionState === "connecting"
                ? t("call.connecting")
                : t("call.participantCount", { count: participants.length })}
            </span>
            <div className="flex -space-x-1 overflow-hidden">
              {participants.slice(0, 4).map((p) => {
                const isSpeaking = speakingUserIds.has(p.user_id);
                return (
                  <div
                    key={p.session_id}
                    title={`${p.display_name}${p.is_muted ? ` (${t("call.muted")})` : ""}`}
                    className={`relative flex size-5 shrink-0 items-center justify-center rounded-full bg-muted text-[10px] font-medium transition-all ${
                      isSpeaking ? "ring-2 ring-emerald-500" : ""
                    }`}
                  >
                    <span>{p.display_name.slice(0, 1).toUpperCase()}</span>
                    {p.is_muted ? (
                      <span className="absolute -bottom-0.5 -right-0.5 rounded-full bg-destructive p-0.5 text-destructive-foreground">
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
