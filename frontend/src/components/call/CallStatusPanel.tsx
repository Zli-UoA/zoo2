import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import {
  assessQuality,
  formatKbps,
  formatMs,
  formatPercent,
  type CallDiagnostics,
  type QualityLevel,
} from "../../lib/call/call-diagnostics";
import { useCall } from "../../lib/call/call-context";

const qualityClass: Record<QualityLevel, string> = {
  good: "bg-emerald-500",
  fair: "bg-amber-500",
  poor: "bg-destructive",
  unknown: "bg-muted-foreground/40",
};

function Row({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-baseline justify-between gap-3 text-xs">
      <dt className="text-muted-foreground">{label}</dt>
      <dd className="min-w-0 truncate text-right font-medium">{value}</dd>
    </div>
  );
}

/** 開いている間だけ 2 秒ごとに診断情報を取得する。 */
export function CallStatusPanel() {
  const { t } = useTranslation();
  const { getCallDiagnostics, participants } = useCall();
  const [diag, setDiag] = useState<CallDiagnostics | null>(null);

  useEffect(() => {
    let active = true;
    const tick = () => {
      void getCallDiagnostics().then((d) => {
        if (active) setDiag(d);
      });
    };
    tick();
    const id = window.setInterval(tick, 2000);
    return () => {
      active = false;
      window.clearInterval(id);
    };
  }, [getCallDiagnostics]);

  if (!diag) return null;
  const nameOf = (userId: string) =>
    participants.find((p) => p.user_id === userId)?.display_name ?? userId;
  const yn = (v: boolean) => (v ? t("call.diagYes") : t("call.diagNo"));
  const heartbeat = diag.lastHeartbeatOkAt
    ? `${Math.max(0, Math.round((diag.collectedAt - diag.lastHeartbeatOkAt) / 1000))}s`
    : "-";

  return (
    <div className="space-y-4">
      <section className="space-y-1.5">
        <h3 className="text-xs font-semibold">{t("call.diagMe")}</h3>
        <dl className="space-y-1">
          <Row label={t("call.diagState")} value={diag.connectionState} />
          <Row label={t("call.diagMuted")} value={yn(diag.isMuted)} />
          <Row
            label={t("call.diagMicTrack")}
            value={[diag.micLabel, diag.micTrackState].filter(Boolean).join(" / ") || "-"}
          />
        </dl>
      </section>
      <section className="space-y-1.5">
        <h3 className="text-xs font-semibold">{t("call.diagServer")}</h3>
        <dl className="space-y-1">
          <Row
            label={t("call.diagServerConn")}
            value={diag.sseConnected ? t("call.diagConnected") : t("call.diagDisconnected")}
          />
          <Row label={t("call.diagHeartbeat")} value={heartbeat} />
          <Row label={t("call.diagHeartbeatFailures")} value={String(diag.heartbeatFailures)} />
          <Row
            label={t("call.diagIce")}
            value={diag.iceServerKinds.map((k) => k.toUpperCase()).join(", ") || "-"}
          />
        </dl>
      </section>
      <section className="space-y-2">
        <h3 className="text-xs font-semibold">{t("call.diagPeers")}</h3>
        {diag.peers.length === 0 ? (
          <p className="text-xs text-muted-foreground">{t("call.diagNoPeers")}</p>
        ) : (
          diag.peers.map((p) => {
            const q = assessQuality(p);
            return (
              <div key={p.sessionId} className="space-y-1 rounded-md border p-2">
                <div className="flex items-center gap-2 text-xs font-medium">
                  <span
                    aria-hidden="true"
                    className={`size-2 shrink-0 rounded-full ${qualityClass[q]}`}
                  />
                  <span className="min-w-0 truncate">{nameOf(p.userId)}</span>
                  <span className="ml-auto text-muted-foreground">
                    {t(
                      `call.quality${q[0].toUpperCase()}${q.slice(1)}` as
                        | "call.qualityGood"
                        | "call.qualityFair"
                        | "call.qualityPoor"
                        | "call.qualityUnknown",
                    )}
                  </span>
                </div>
                <dl className="space-y-0.5">
                  <Row
                    label={t("call.diagState")}
                    value={`${p.connectionState} / ${p.iceConnectionState}`}
                  />
                  <Row label={t("call.diagRoute")} value={p.candidateType ?? "-"} />
                  <Row label={t("call.diagRtt")} value={formatMs(p.rttMs)} />
                  <Row label={t("call.diagLoss")} value={formatPercent(p.lossRatio)} />
                  <Row label={t("call.diagJitter")} value={formatMs(p.jitterMs)} />
                  <Row label={t("call.diagInbound")} value={formatKbps(p.inboundKbps)} />
                  <Row label={t("call.diagOutbound")} value={formatKbps(p.outboundKbps)} />
                </dl>
              </div>
            );
          })
        )}
      </section>
    </div>
  );
}
