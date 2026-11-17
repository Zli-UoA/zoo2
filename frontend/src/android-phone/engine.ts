import { AudioCallMedia } from "../lib/call/audio-media";

declare global {
  interface Window {
    ZooNativePhone: { postMessage(message: string): void };
    ZooPhone: { execute(id: number, token: string, method: string, payload: unknown): void };
  }
}
let token = "";
let media: AudioCallMedia | null = null;
const emit = (event: string, payload?: unknown) =>
  window.ZooNativePhone.postMessage(JSON.stringify({ token, event, payload }));
window.ZooPhone = {
  execute(id, nextToken, method, payload) {
    if (token && nextToken !== token) return;
    token = nextToken;
    void (async () => {
      const value = payload as Record<string, unknown>;
      if (method === "rtc.start") {
        media?.close();
        const next = new AudioCallMedia(
          value.session_id as string,
          value.ice_servers as RTCIceServer[],
          emit,
          undefined,
          value.relay === true,
        );
        media = next;
        await next.start();
        if (media === next && value.room_state)
          await next.event("room_state", value.room_state as Record<string, unknown>);
      } else if (method === "rtc.event") {
        if (media) await media.event(value.event as string, value.data as Record<string, unknown>);
      } else if (method === "mute") {
        media?.mute(value.muted === true);
      } else if (method === "close") {
        media?.close();
        media = null;
      } else throw new Error("Unknown phone command");
      emit("result", { id, ok: true });
    })().catch(() => emit("result", { id, ok: false }));
  },
};
window.ZooNativePhone.postMessage(JSON.stringify({ event: "loaded" }));
