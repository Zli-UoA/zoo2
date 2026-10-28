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

export type usizeNumber = number;

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
