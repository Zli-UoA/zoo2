export interface SipPhone {
  id?: string;
  name: string;
  extension: string;
}
export interface SipLine {
  id: string;
  name: string;
  shared: boolean;
  owner_user_id: string | null;
  websocket_path: string;
  sip_domain: string;
  username: string;
  phone_number: string;
  member_ids: string[];
  phones: SipPhone[];
  can_manage: boolean;
  can_connect: boolean;
}
export interface SipDirectory {
  enabled: boolean;
  lines: SipLine[];
}
export interface SipLineInput {
  name: string;
  shared: boolean;
  websocket_path: string;
  sip_domain: string;
  username: string;
  password?: string;
  phone_number: string;
  member_ids: string[];
  phones: SipPhone[];
}
export interface PhoneConnection {
  relay_only?: boolean;
  session_id: string;
  peer_id: string;
  ice_servers: RTCIceServer[];
  revision: string;
}

export type SipStatus = "offline" | "connecting" | "ready" | "dialing" | "incoming" | "active";
export interface SipState {
  status: SipStatus;
  lineId: string | null;
  muted: boolean;
  busy: boolean;
  error: boolean;
  peer: string;
}
