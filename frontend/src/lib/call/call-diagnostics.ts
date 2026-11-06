/**
 * 通話の通信状況 (RTCStatsReport) の集計と整形。
 */

export interface PeerStatsSample {
  /** 計測時刻 (ms) */
  at: number;
  bytesReceived: number;
  bytesSent: number;
}

export interface PeerDiagnostics {
  sessionId: string;
  userId: string;
  connectionState: string;
  iceConnectionState: string;
  /** 往復遅延 (ms)。取得できなければ null */
  rttMs: number | null;
  /** 受信パケットロス率 (0-1) */
  lossRatio: number | null;
  /** 受信ジッタ (ms) */
  jitterMs: number | null;
  /** 受信/送信ビットレート (kbps)。前回サンプルが無ければ null */
  inboundKbps: number | null;
  outboundKbps: number | null;
  /** 選択された経路: host / srflx / prflx / relay */
  candidateType: string | null;
  sample: PeerStatsSample;
}

export interface CallDiagnostics {
  collectedAt: number;
  connectionState: string;
  isMuted: boolean;
  micTrackState: string | null;
  micLabel: string | null;
  sseConnected: boolean;
  lastHeartbeatOkAt: number | null;
  heartbeatFailures: number;
  iceServerKinds: ("stun" | "turn")[];
  peers: PeerDiagnostics[];
}

export function kindsOfIceServers(servers: RTCIceServer[]): ("stun" | "turn")[] {
  const kinds = new Set<"stun" | "turn">();
  for (const s of servers) {
    const urls = Array.isArray(s.urls) ? s.urls : [s.urls];
    for (const u of urls) {
      if (/^turns?:/i.test(u)) kinds.add("turn");
      else if (/^stuns?:/i.test(u)) kinds.add("stun");
    }
  }
  return [...kinds];
}

function kbps(prevBytes: number, nowBytes: number, prevAt: number, nowAt: number): number | null {
  const dt = (nowAt - prevAt) / 1000;
  if (dt <= 0 || nowBytes < prevBytes) return null;
  return ((nowBytes - prevBytes) * 8) / 1000 / dt;
}

/** RTCStatsReport から主要な値を取り出す。 */
export function summarizeStats(
  report: Iterable<Record<string, unknown>>,
  at: number,
  prev?: PeerStatsSample,
): Pick<
  PeerDiagnostics,
  "rttMs" | "lossRatio" | "jitterMs" | "inboundKbps" | "outboundKbps" | "candidateType" | "sample"
> {
  const stats = [...report];
  const byId = new Map<string, Record<string, unknown>>();
  for (const s of stats) if (typeof s.id === "string") byId.set(s.id, s);

  let rttMs: number | null = null;
  let candidateType: string | null = null;
  let lossRatio: number | null = null;
  let jitterMs: number | null = null;
  let bytesReceived = 0;
  let bytesSent = 0;

  for (const s of stats) {
    if (s.type === "candidate-pair" && (s.nominated === true || s.state === "succeeded")) {
      if (typeof s.currentRoundTripTime === "number") rttMs = s.currentRoundTripTime * 1000;
      const local = typeof s.localCandidateId === "string" ? byId.get(s.localCandidateId) : null;
      if (local && typeof local.candidateType === "string") candidateType = local.candidateType;
      const remote = typeof s.remoteCandidateId === "string" ? byId.get(s.remoteCandidateId) : null;
      if (remote?.candidateType === "relay") candidateType = "relay";
    } else if (s.type === "inbound-rtp" && s.kind === "audio") {
      const received = typeof s.packetsReceived === "number" ? s.packetsReceived : 0;
      const lost = typeof s.packetsLost === "number" ? Math.max(0, s.packetsLost) : 0;
      if (received + lost > 0) lossRatio = lost / (received + lost);
      if (typeof s.jitter === "number") jitterMs = s.jitter * 1000;
      if (typeof s.bytesReceived === "number") bytesReceived += s.bytesReceived;
    } else if (s.type === "outbound-rtp" && s.kind === "audio") {
      if (typeof s.bytesSent === "number") bytesSent += s.bytesSent;
    } else if (s.type === "remote-inbound-rtp" && rttMs === null) {
      if (typeof s.roundTripTime === "number") rttMs = s.roundTripTime * 1000;
    }
  }

  const sample = { at, bytesReceived, bytesSent };
  return {
    rttMs,
    lossRatio,
    jitterMs,
    inboundKbps: prev ? kbps(prev.bytesReceived, bytesReceived, prev.at, at) : null,
    outboundKbps: prev ? kbps(prev.bytesSent, bytesSent, prev.at, at) : null,
    candidateType,
    sample,
  };
}

export type QualityLevel = "good" | "fair" | "poor" | "unknown";

/** RTT・ロス率から大まかな品質を判定する。 */
export function assessQuality(p: Pick<PeerDiagnostics, "rttMs" | "lossRatio">): QualityLevel {
  if (p.rttMs === null && p.lossRatio === null) return "unknown";
  const rtt = p.rttMs ?? 0;
  const loss = p.lossRatio ?? 0;
  if (rtt > 400 || loss > 0.1) return "poor";
  if (rtt > 150 || loss > 0.03) return "fair";
  return "good";
}

export function formatMs(v: number | null): string {
  return v === null ? "-" : `${Math.round(v)} ms`;
}
export function formatPercent(v: number | null): string {
  return v === null ? "-" : `${(v * 100).toFixed(1)}%`;
}
export function formatKbps(v: number | null): string {
  return v === null ? "-" : `${v.toFixed(0)} kbps`;
}
