import { describe, expect, it, vi } from "vite-plus/test";

const state = vi.hoisted(() => ({
  log: [] as string[],
  publishGate: null as Promise<void> | null,
}));

vi.mock("livekit-client", () => {
  class LocalAudioTrack {
    isMuted = false;
    constructor(readonly mediaStreamTrack: MediaStreamTrack) {}
    async mute() {
      this.isMuted = true;
      state.log.push("mute");
      return this;
    }
    async unmute() {
      this.isMuted = false;
      state.log.push("unmute");
      return this;
    }
    async replaceTrack() {
      return this;
    }
  }
  class Room {
    localParticipant = {
      publishTrack: async (track: LocalAudioTrack) => {
        state.log.push(`publish muted=${track.isMuted}`);
        if (state.publishGate) await state.publishGate;
        return { track };
      },
    };
    on() {}
    async connect() {}
    async disconnect() {}
  }
  return {
    LocalAudioTrack,
    Room,
    RoomEvent: { TrackSubscribed: "s", TrackUnsubscribed: "u", Disconnected: "d" },
    Track: { Kind: { Audio: "audio" }, Source: { Microphone: "microphone" } },
  };
});

const { LiveKitAdapter } = await import("./livekit-adapter");

const noCallbacks = { onRemoteAudio: () => {}, onRemoteGone: () => {}, onFailed: () => {} };

describe("LiveKitAdapter mute", () => {
  it("applies a mute requested before start() before publishing", async () => {
    state.log = [];
    state.publishGate = null;
    const adapter = new LiveKitAdapter("wss://x", "t", {} as MediaStreamTrack, noCallbacks);
    await adapter.setMuted(true);
    await adapter.start();
    expect(state.log).toEqual(["mute", "publish muted=true"]);
    adapter.close();
  });

  it("publishes muted when created with the current muted state", async () => {
    state.log = [];
    state.publishGate = null;
    // ミュート中の mesh 通話から移るとき、call-context は作るときに今の状態を渡す（start() の前に setMuted しない）。
    const adapter = new LiveKitAdapter("wss://x", "t", {} as MediaStreamTrack, noCallbacks, {
      muted: true,
    });
    await adapter.start();
    expect(state.log).toEqual(["mute", "publish muted=true"]);
    adapter.close();
  });

  it("applies a mute change made while publishing", async () => {
    state.log = [];
    let release = () => {};
    state.publishGate = new Promise<void>((resolve) => (release = resolve));
    const adapter = new LiveKitAdapter("wss://x", "t", {} as MediaStreamTrack, noCallbacks);
    const started = adapter.start();
    await vi.waitFor(() => expect(state.log).toEqual(["publish muted=false"]));
    await adapter.setMuted(true);
    release();
    await started;
    expect(state.log).toEqual(["publish muted=false", "mute"]);
    adapter.close();
  });
});
