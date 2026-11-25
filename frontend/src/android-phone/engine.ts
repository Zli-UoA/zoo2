import { AudioCallMedia } from "../lib/call/audio-media";
import { AndroidSfuCall } from "./sfu-call";

declare global {
  interface Window {
    ZooNativePhone: { postMessage(message: string): void };
    ZooPhone: { execute(id: number, token: string, method: string, payload: unknown): void };
  }
}
let token = "";
let media: AudioCallMedia | null = null;
// SFU モード（docs/calls-sfu.md）。切り替えたら、mesh の media にはイベントを渡さない。
let sfu: AndroidSfuCall | null = null;
let sfuMode = false;
let session = "";
const participants = new Map<string, { session_id: string }>();
let audioLevelTimer: ReturnType<typeof setTimeout> | undefined;
const emit = (event: string, payload?: unknown) =>
  window.ZooNativePhone.postMessage(JSON.stringify({ token, event, payload }));
function watchRemoteAudio(current: AudioCallMedia | AndroidSfuCall, userId?: string) {
  let previous = 0;
  async function sample() {
    const level =
      current instanceof AudioCallMedia
        ? Math.round((await current.remoteAudioLevel(userId)) * 100) / 100
        : Math.round((await current.remoteAudioLevel()) * 100) / 100;
    if (media !== current && sfu !== current) return;
    if (level !== previous) emit("remote_audio_level", { level });
    previous = level;
    audioLevelTimer = setTimeout(() => void sample(), 100);
  }
  audioLevelTimer = setTimeout(() => void sample(), 100);
}
function trackParticipants(event: string, data: Record<string, unknown>) {
  if (event === "room_state" && Array.isArray(data.participants)) {
    participants.clear();
    for (const p of data.participants as { session_id?: string }[])
      if (typeof p.session_id === "string")
        participants.set(p.session_id, { session_id: p.session_id });
  } else if (event === "participant_joined") {
    const p = data.participant as { session_id?: string } | undefined;
    if (typeof p?.session_id === "string")
      participants.set(p.session_id, { session_id: p.session_id });
  } else if (event === "participant_left" && typeof data.session_id === "string") {
    participants.delete(data.session_id);
  }
}
async function switchToSfu() {
  if (sfuMode) return;
  sfuMode = true;
  // mesh のマイクを複製して使い、SFU につながったら mesh を閉じる。
  const source = media?.micTrack();
  const mic = source?.clone();
  if (!source || !mic) {
    emit("failed");
    return;
  }
  // 複製にもミュートの状態を引き継ぐ。
  mic.enabled = source.enabled;
  const next = new AndroidSfuCall(session, mic, emit, [...participants.values()]);
  sfu = next;
  try {
    await next.start();
  } catch {
    // つながらなければ、通話の失敗にする（mesh は閉じていない）。
    if (sfu === next) emit("failed");
    return;
  }
  if (sfu !== next) return;
  if (audioLevelTimer !== undefined) clearTimeout(audioLevelTimer);
  media?.close();
  media = null;
  watchRemoteAudio(next);
}
async function dispatch(event: string, data: Record<string, unknown>) {
  trackParticipants(event, data);
  if ((event === "room_state" || event === "mode_changed") && data.mode === "sfu") {
    // 接続の成立まで待つので、ここでは待たない（Kotlin の rtc.event の時間制限と、後続のイベントを止めないため）。
    void switchToSfu();
    return;
  }
  if (sfuMode) sfu?.event(event, data);
  else if (media) await media.event(event, data);
}
window.ZooPhone = {
  execute(id, nextToken, method, payload) {
    if (token && nextToken !== token) return;
    token = nextToken;
    void (async () => {
      const value = payload as Record<string, unknown>;
      if (method === "rtc.start") {
        if (audioLevelTimer !== undefined) clearTimeout(audioLevelTimer);
        media?.close();
        sfu?.close();
        sfu = null;
        sfuMode = false;
        participants.clear();
        session = value.session_id as string;
        const next = new AudioCallMedia(
          value.session_id as string,
          value.ice_servers as RTCIceServer[],
          (event, payload) => {
            emit(event, payload);
            if (event === "connected" && media === next)
              watchRemoteAudio(
                next,
                typeof value.peer_user_id === "string" ? value.peer_user_id : undefined,
              );
          },
          undefined,
          value.relay === true,
        );
        media = next;
        await next.start();
        if (media === next && value.room_state)
          await dispatch("room_state", value.room_state as Record<string, unknown>);
      } else if (method === "rtc.event") {
        await dispatch(value.event as string, value.data as Record<string, unknown>);
      } else if (method === "rtc.sfu.reply") {
        sfu?.reply(value);
      } else if (method === "mute") {
        media?.mute(value.muted === true);
        sfu?.mute(value.muted === true);
      } else if (method === "close") {
        if (audioLevelTimer !== undefined) clearTimeout(audioLevelTimer);
        media?.close();
        media = null;
        sfu?.close();
        sfu = null;
      } else throw new Error("Unknown phone command");
      emit("result", { id, ok: true });
    })().catch(() => emit("result", { id, ok: false }));
  },
};
window.ZooNativePhone.postMessage(JSON.stringify({ event: "loaded" }));
