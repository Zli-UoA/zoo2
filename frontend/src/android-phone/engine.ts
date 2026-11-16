import { createSipDriver } from "../lib/sip/driver";
import { dialTarget, type SipDriver } from "../lib/sip/controller";
import type { SipCredentials } from "../lib/sip/types";
import { DmMedia } from "./dm-media";

declare global {
  interface Window {
    ZooNativePhone: { postMessage(message: string): void };
    ZooPhone: { execute(id: number, token: string, method: string, payload: unknown): void };
  }
}
let token = "";
let generation = 0;
let sip: SipDriver | null = null;
let dm: DmMedia | null = null;
let domain = "";
const emit = (event: string, payload?: unknown) =>
  window.ZooNativePhone.postMessage(JSON.stringify({ token, event, payload }));
window.ZooPhone = {
  execute(id, nextToken, method, payload) {
    if (token && nextToken !== token) return;
    token = nextToken;
    void (async () => {
      const value = payload as Record<string, unknown>;
      if (method === "sip.register") {
        const mine = ++generation;
        const events = (event: string) => () => {
          if (mine === generation) emit(event);
        };
        const driver = await createSipDriver(payload as SipCredentials, {
          ready: events("registered"),
          incoming: events("incoming"),
          answered: events("connected"),
          ended: events("ended"),
          disconnected: events("failed"),
        });
        if (mine !== generation) {
          await driver.disconnect();
          return;
        }
        sip = driver;
        domain = (payload as SipCredentials).sip_domain;
        await driver.connect();
      } else if (method === "sip.call") {
        if (!sip) throw new Error();
        await sip.call(dialTarget(value.number as string, domain));
      } else if (method === "sip.answer") {
        if (!sip) throw new Error();
        await sip.answer();
      } else if (method === "sip.end") {
        if (sip) await sip.hangup();
      } else if (method === "sip.dtmf") {
        if (sip && /^[0-9*#]$/.test(value.tone as string)) await sip.dtmf(value.tone as string);
      } else if (method === "dm.start") {
        dm = new DmMedia(value.session_id as string, value.ice_servers as RTCIceServer[], emit);
        await dm.start();
      } else if (method === "dm.event") {
        if (dm) await dm.event(value.event as string, value.data as Record<string, unknown>);
      } else if (method === "mute") {
        sip?.mute(value.muted === true);
        dm?.mute(value.muted === true);
      } else if (method === "close") {
        ++generation;
        const previous = sip;
        sip = null;
        dm?.close();
        dm = null;
        await previous?.disconnect();
      } else throw new Error("Unknown phone command");
      emit("result", { id, ok: true });
    })().catch(() => emit("result", { id, ok: false }));
  },
};
window.ZooNativePhone.postMessage(JSON.stringify({ event: "loaded" }));
