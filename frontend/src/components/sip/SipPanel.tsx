import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { useCurrentUser } from "../../hooks/useCurrentUser";
import { useUsers } from "../../hooks/useRealtime";
import { primeCallSounds } from "../../lib/call/call-sounds";
import { useSip } from "./SipProvider";
import { SipPanelView } from "./SipPanelView";
import { SipControls } from "./SipControls";

export function SipPanel() {
  const sip = useSip();
  const { isAdmin, currentUserId } = useCurrentUser();
  const { data: users } = useUsers();
  const refresh = sip?.refresh;
  const loaded = sip?.loaded;
  useEffect(() => {
    if (!loaded) void refresh?.();
  }, [refresh, loaded]);
  if (!sip) return null;
  const c = sip.controller;
  return (
    <SipPanelView
      key={currentUserId}
      directory={sip.directory}
      state={sip.state}
      loading={sip.loading}
      loadError={sip.loadError}
      isAdmin={isAdmin}
      channelBusy={sip.channelBusy}
      users={(users ?? [])
        .filter((u) => !u.is_bot && !u.is_deactivated)
        .map((u) => ({ id: u.id, name: u.display_name || u.user_name }))}
      onRefresh={() => void sip.refresh()}
      onConnect={(id) => {
        primeCallSounds();
        void c.connect(id);
      }}
      onDisconnect={c.disconnect}
      onDial={(number) => void c.dial(number)}
      onAnswer={() => void c.answer()}
      onHangup={() => void c.hangup()}
      onMute={c.toggleMute}
      onDtmf={(tone) => void c.dtmf(tone)}
      onSave={sip.save}
      onRemove={sip.remove}
    />
  );
}
/** Always mounted beside the shell so navigation cannot hide incoming/ongoing calls. */
export function SipCallBanner() {
  const { t } = useTranslation();
  const sip = useSip();
  if (!sip || !["incoming", "dialing", "active"].includes(sip.state.status)) return null;
  const c = sip.controller;
  const line = sip.directory.lines.find((l) => l.id === sip.state.lineId);
  return (
    <section
      aria-label={t("sip.title")}
      className="fixed right-3 bottom-3 z-50 max-h-[80dvh] w-80 max-w-[calc(100vw-1.5rem)] space-y-2 overflow-auto rounded-lg border bg-background p-4 shadow-lg"
    >
      <h2 className="break-words font-medium">{line?.name ?? t("sip.title")}</h2>
      <SipControls
        state={sip.state}
        onAnswer={() => void c.answer()}
        onHangup={() => void c.hangup()}
        onMute={c.toggleMute}
        onDtmf={(tone) => void c.dtmf(tone)}
      />
      <button type="button" className="text-sm text-link underline" onClick={c.disconnect}>
        {t("sip.disconnect")}
      </button>
    </section>
  );
}
