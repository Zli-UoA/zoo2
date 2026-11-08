import { describe, expect, it } from "vite-plus/test";

import {
  assessQuality,
  formatKbps,
  formatMs,
  formatPercent,
  kindsOfIceServers,
  summarizeStats,
} from "./call-diagnostics";

describe("kindsOfIceServers", () => {
  it("stun と turn を判別する", () => {
    expect(kindsOfIceServers([{ urls: "stun:a" }, { urls: ["turns:b", "turn:c"] }]).sort()).toEqual(
      ["stun", "turn"],
    );
  });
  it("空なら空", () => {
    expect(kindsOfIceServers([])).toEqual([]);
  });
});

describe("summarizeStats", () => {
  const report = (bytesReceived: number, bytesSent: number) => [
    {
      id: "cp",
      type: "candidate-pair",
      nominated: true,
      currentRoundTripTime: 0.05,
      localCandidateId: "l",
      remoteCandidateId: "r",
    },
    { id: "l", type: "local-candidate", candidateType: "srflx" },
    { id: "r", type: "remote-candidate", candidateType: "host" },
    {
      id: "in",
      type: "inbound-rtp",
      kind: "audio",
      packetsReceived: 90,
      packetsLost: 10,
      jitter: 0.02,
      bytesReceived,
    },
    { id: "out", type: "outbound-rtp", kind: "audio", bytesSent },
  ];
  it("RTT・ロス・ジッタ・経路を取り出す", () => {
    const s = summarizeStats(report(1000, 500), 1000);
    expect(s.rttMs).toBeCloseTo(50);
    expect(s.lossRatio).toBeCloseTo(0.1);
    expect(s.jitterMs).toBeCloseTo(20);
    expect(s.candidateType).toBe("srflx");
    expect(s.inboundKbps).toBeNull();
  });
  it("前回サンプルからビットレートを計算する", () => {
    const first = summarizeStats(report(0, 0), 0);
    const second = summarizeStats(report(25000, 12500), 2000, first.sample);
    expect(second.inboundKbps).toBeCloseTo(100);
    expect(second.outboundKbps).toBeCloseTo(50);
  });
  it("relay を検出する", () => {
    const r = report(0, 0);
    r[2] = { id: "r", type: "remote-candidate", candidateType: "relay" };
    expect(summarizeStats(r, 0).candidateType).toBe("relay");
  });
});

describe("使用中の候補ペア", () => {
  it("transport が選択したペアのみから経路と RTT を取る", () => {
    const r = [
      { id: "t", type: "transport", selectedCandidatePairId: "cp1" },
      {
        id: "cp1",
        type: "candidate-pair",
        state: "succeeded",
        currentRoundTripTime: 0.025,
        localCandidateId: "l1",
        remoteCandidateId: "r1",
      },
      {
        id: "cp2",
        type: "candidate-pair",
        state: "succeeded",
        currentRoundTripTime: 0.8,
        localCandidateId: "l2",
        remoteCandidateId: "r2",
      },
      { id: "l1", type: "local-candidate", candidateType: "host" },
      { id: "r1", type: "remote-candidate", candidateType: "host" },
      { id: "l2", type: "local-candidate", candidateType: "relay" },
      { id: "r2", type: "remote-candidate", candidateType: "relay" },
    ];
    const s = summarizeStats(r, 0);
    expect(s.rttMs).toBeCloseTo(25);
    expect(s.candidateType).toBe("host");
  });
});

describe("assessQuality / format", () => {
  it("判定", () => {
    expect(assessQuality({ rttMs: null, lossRatio: null })).toBe("unknown");
    expect(assessQuality({ rttMs: 40, lossRatio: 0 })).toBe("good");
    expect(assessQuality({ rttMs: 200, lossRatio: 0 })).toBe("fair");
    expect(assessQuality({ rttMs: 40, lossRatio: 0.2 })).toBe("poor");
  });
  it("整形", () => {
    expect(formatMs(null)).toBe("-");
    expect(formatMs(12.4)).toBe("12 ms");
    expect(formatPercent(0.125)).toBe("12.5%");
    expect(formatKbps(31.6)).toBe("32 kbps");
  });
});
