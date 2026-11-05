export interface PublicCallParticipant {
  user_id: string;
  user_name: string;
  display_name: string;
  is_muted: boolean;
  joined_at: string;
}

export interface CallParticipant extends PublicCallParticipant {
  session_id: string;
}

export interface CallStatusResponse {
  is_active: boolean;
  channel_id: string;
  participant_count: number;
  participants: PublicCallParticipant[];
  max_participants: number;
  ice_servers: RTCIceServer[];
}

export function isPublicCallParticipant(value: unknown): value is PublicCallParticipant {
  if (!value || typeof value !== "object") return false;
  const p = value as Record<string, unknown>;
  return (
    typeof p.user_id === "string" &&
    typeof p.user_name === "string" &&
    typeof p.display_name === "string" &&
    typeof p.is_muted === "boolean" &&
    typeof p.joined_at === "string"
  );
}

export function isCallParticipant(value: unknown): value is CallParticipant {
  if (!isPublicCallParticipant(value)) return false;
  const p = value as unknown as Record<string, unknown>;
  return typeof p.session_id === "string";
}

export function isActiveCallSummary(value: unknown): value is ActiveCallSummary {
  if (!value || typeof value !== "object") return false;
  const s = value as Record<string, unknown>;
  return (
    typeof s.channel_id === "string" &&
    typeof s.participant_count === "number" &&
    Array.isArray(s.participants) &&
    s.participants.every(isPublicCallParticipant)
  );
}

export interface ActiveCallSummary {
  channel_id: string;
  participant_count: number;
  participants: PublicCallParticipant[];
}

export type CallConnectionState = "idle" | "connecting" | "connected" | "error";

export interface CallContextValue {
  currentChannelId: string | null;
  connectionState: CallConnectionState;
  error: string | null;
  participants: CallParticipant[];
  isMuted: boolean;
  speakingUserIds: Set<string>;
  activeCalls: Map<string, number>;
  joinCall: (channelId: string) => Promise<void>;
  leaveCall: () => Promise<void>;
  toggleMute: () => Promise<void>;
}
