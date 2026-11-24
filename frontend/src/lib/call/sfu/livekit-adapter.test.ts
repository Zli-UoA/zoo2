import { describe, expect, it, vi } from "vite-plus/test";

const state = vi.hoisted(() => ({
  log: [] as string[],
  publishGate: null as Promise<void> | null,
  replaceError: null as Error | null,
}));

vi.mock("livekit-client", () => {
  class LocalAudioTrack {
    isMuted = false;
    constructor(readonly mediaStreamTrack: MediaStreamTrack) {
      // SDK と同じく、包むときにトラックを有効にする。
      mediaStreamTrack.enabled = true;
    }
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
      if (state.replaceError) throw state.replaceError;
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

/** mesh と共有するマイクの代わり。clone() は LiveKit に渡す複製を返す。 */
function fakeMic(enabled = true) {
  const clones: { enabled: boolean; stopped: boolean }[] = [];
  const mic = {
    enabled,
    clone() {
      const c = {
        enabled: mic.enabled,
        stopped: false,
        clone: () => c,
        stop() {
          c.stopped = true;
        },
      };
      clones.push(c);
      return c;
    },
    stop() {},
  };
  return { mic: mic as unknown as MediaStreamTrack, clones, raw: mic };
}

describe("LiveKitAdapter mute", () => {
  it("applies a mute requested before start() before publishing", async () => {
    state.log = [];
    state.publishGate = null;
    const adapter = new LiveKitAdapter("wss://x", "t", fakeMic().mic, noCallbacks);
    await adapter.setMuted(true);
    await adapter.start();
    expect(state.log).toEqual(["mute", "publish muted=true"]);
    adapter.close();
  });

  it("publishes muted when created with the current muted state", async () => {
    state.log = [];
    state.publishGate = null;
    // ミュート中の mesh 通話から移るとき、call-context は作るときに今の状態を渡す（start() の前に setMuted しない）。
    const adapter = new LiveKitAdapter("wss://x", "t", fakeMic().mic, noCallbacks, {
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
    const adapter = new LiveKitAdapter("wss://x", "t", fakeMic().mic, noCallbacks);
    const started = adapter.start();
    await vi.waitFor(() => expect(state.log).toEqual(["publish muted=false"]));
    await adapter.setMuted(true);
    release();
    await started;
    expect(state.log).toEqual(["publish muted=false", "mute"]);
    adapter.close();
  });

  it("does not enable the mic shared with mesh while muted", async () => {
    state.log = [];
    state.publishGate = null;
    // ミュート中の mesh のマイク（enabled = false）。SDK が包むのは複製なので、mesh のマイクは有効にならない。
    const { mic, clones, raw } = fakeMic(false);
    const adapter = new LiveKitAdapter("wss://x", "t", mic, noCallbacks, { muted: true });
    await adapter.start();
    expect(raw.enabled).toBe(false);
    expect(clones).toHaveLength(1);
    adapter.close();
    // 止めるのは複製だけ。
    expect(clones[0].stopped).toBe(true);
  });

  it("stops sending from the clone when muted while publishing", async () => {
    state.log = [];
    let release = () => {};
    state.publishGate = new Promise<void>((resolve) => (release = resolve));
    const { mic, clones } = fakeMic(true);
    const adapter = new LiveKitAdapter("wss://x", "t", mic, noCallbacks);
    const started = adapter.start();
    await vi.waitFor(() => expect(state.log).toEqual(["publish muted=false"]));
    // publish が終わる前にミュートしたら、その時点で複製の送信を止める。
    await adapter.setMuted(true);
    expect(clones[0].enabled).toBe(false);
    release();
    await started;
    expect(state.log).toEqual(["publish muted=false", "mute"]);
    adapter.close();
  });

  it("keeps and later stops the current clone when replacing the mic fails", async () => {
    state.log = [];
    state.publishGate = null;
    const first = fakeMic();
    const adapter = new LiveKitAdapter("wss://x", "t", first.mic, noCallbacks);
    await adapter.start();
    const second = fakeMic();
    state.replaceError = new Error("replace failed");
    try {
      await expect(adapter.replaceMicTrack(second.mic)).rejects.toThrow("replace failed");
    } finally {
      state.replaceError = null;
    }
    // 差し替えられなかった複製は止め、使っている複製は残す。
    expect(second.clones[0].stopped).toBe(true);
    expect(first.clones[0].stopped).toBe(false);
    // 退出すると、使っていた複製を止める。
    adapter.close();
    expect(first.clones[0].stopped).toBe(true);
  });
});
