import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { SipState } from "../../lib/sip/types";
import { Button } from "../ui/button";
import { Input } from "../ui/input";

export interface SipControlsProps {
  state: SipState;
  onAnswer(this: void): void;
  onHangup(this: void): void;
  onMute(this: void): void;
  onDtmf(this: void, tone: string): void;
}
export function SipControls({ state, onAnswer, onHangup, onMute, onDtmf }: SipControlsProps) {
  const { t } = useTranslation();
  const [tone, setTone] = useState("");
  return (
    <div className="space-y-3">
      <p role="status" className="break-all text-sm">
        {t(`sip.status.${state.status}`)}
        {state.peer ? ` · ${state.peer}` : ""}
      </p>
      {state.error ? (
        <p role="alert" className="text-sm text-destructive">
          {t("sip.callError")}
        </p>
      ) : null}
      <div className="flex flex-wrap gap-2">
        {state.status === "incoming" ? (
          <Button disabled={state.busy} onClick={onAnswer}>
            {t("sip.answer")}
          </Button>
        ) : null}
        {["dialing", "incoming", "active"].includes(state.status) ? (
          <Button variant="destructive" disabled={state.busy} onClick={onHangup}>
            {t("sip.hangup")}
          </Button>
        ) : null}
        {state.status === "active" ? (
          <Button
            variant="outline"
            disabled={state.busy}
            aria-pressed={state.muted}
            onClick={onMute}
          >
            {t(state.muted ? "sip.unmute" : "sip.mute")}
          </Button>
        ) : null}
      </div>
      {state.status === "active" ? (
        <form
          className="flex flex-wrap items-end gap-2"
          onSubmit={(e) => {
            e.preventDefault();
            if (/^[0-9*#]$/.test(tone)) onDtmf(tone);
          }}
        >
          <label className="grid gap-1 text-sm">
            {t("sip.dtmf")}
            <Input
              value={tone}
              onChange={(e) => setTone(e.target.value)}
              maxLength={1}
              pattern="[0-9*#]"
              className="w-24"
            />
          </label>
          <Button type="submit" variant="outline" disabled={state.busy || !/^[0-9*#]$/.test(tone)}>
            {t("sip.sendTone")}
          </Button>
        </form>
      ) : null}
    </div>
  );
}
