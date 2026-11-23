import { afterEach, describe, expect, it, vi } from "vite-plus/test";

import {
  CF_CONNECT_TIMEOUT_MS,
  CF_ICE_GATHERING_TIMEOUT_MS,
  CfRealtimeAdapter,
  type CfTracksResponse,
  enableOpusDtx,
  speakerOfTrackName,
  type CfRealtimeApi,
} from "./cf-realtime-adapter";
import { speakerOf } from "./livekit-adapter";
import { isSfuJoinResponse } from "./types";

describe("enableOpusDtx", () => {
  it("adds usedtx to the Opus fmtp line", () => {
    const sdp = "m=audio 9 X 111\r\na=rtpmap:111 opus/48000/2\r\na=fmtp:111 minptime=10\r\n";
    expect(enableOpusDtx(sdp)).toContain("a=fmtp:111 minptime=10;usedtx=1");
  });

  it("leaves an SDP without Opus or with DTX already on", () => {
    const pcmu = "m=audio 9 X 0\r\na=rtpmap:0 PCMU/8000\r\n";
    expect(enableOpusDtx(pcmu)).toBe(pcmu);
    const dtx = "a=rtpmap:111 opus/48000/2\r\na=fmtp:111 usedtx=1\r\n";
    expect(enableOpusDtx(dtx)).toBe(dtx);
  });
});

describe("speaker mapping", () => {
  it("uses the track name for bridge tracks", () => {
    expect(speakerOfTrackName("abc-bridge")).toBe("abc");
    expect(speakerOfTrackName("abc")).toBe("abc");
    expect(speakerOf("bridge:12", "sess-1")).toBe("sess-1");
    expect(speakerOf("sess-2", "mic")).toBe("sess-2");
  });
});

describe("isSfuJoinResponse", () => {
  it("accepts both SFUs", () => {
    expect(isSfuJoinResponse({ sfu: "cloudflare", epoch: 1, version: 2, tracks: [] })).toBe(true);
    expect(
      isSfuJoinResponse({
        sfu: "livekit",
        url: "wss://x",
        token: "t",
        epoch: 1,
        version: 0,
        tracks: [{ id: "s:direct", session_id: "s", source: "direct" }],
      }),
    ).toBe(true);
  });

  it("rejects malformed responses", () => {
    expect(isSfuJoinResponse({ sfu: "livekit", epoch: 1, version: 0, tracks: [] })).toBe(false);
    expect(isSfuJoinResponse({ sfu: "kubecamp", epoch: 1, version: 0, tracks: [] })).toBe(false);
    expect(
      isSfuJoinResponse({ sfu: "cloudflare", epoch: 1, version: 0, tracks: [{ id: 1 }] }),
    ).toBe(false);
  });
});

class FakePeerConnection {
  static last: FakePeerConnection | null = null;
  /** setRemoteDescription(answer) でつながるか（false なら呼び出し側で状態を変える）。 */
  static autoConnect = true;
  /** setLocalDescription で候補を集め終えるか。 */
  static autoGather = true;
  listeners = new Map<string, ((e: unknown) => void)[]>();
  connectionState = "new";
  iceGatheringState = "new";
  localDescription: RTCSessionDescriptionInit | null = null;
  local: RTCSessionDescriptionInit | null = null;
  remote: RTCSessionDescriptionInit[] = [];
  added: MediaStreamTrack[] = [];
  closed = false;
  constructor() {
    FakePeerConnection.last = this;
  }
  addEventListener(type: string, fn: (e: unknown) => void) {
    this.listeners.set(type, [...(this.listeners.get(type) ?? []), fn]);
  }
  removeEventListener(type: string, fn: (e: unknown) => void) {
    this.listeners.set(
      type,
      (this.listeners.get(type) ?? []).filter((f) => f !== fn),
    );
  }
  fire(type: string) {
    for (const fn of this.listeners.get(type) ?? []) fn({});
  }
  setConnectionState(state: string) {
    this.connectionState = state;
    this.fire("connectionstatechange");
  }
  finishGathering() {
    if (!this.local) return;
    this.iceGatheringState = "complete";
    this.localDescription = { ...this.local, sdp: `${this.local.sdp}a=candidate:1\r\n` };
    this.fire("icegatheringstatechange");
  }
  addTransceiver(track: MediaStreamTrack) {
    this.added.push(track);
    return { sender: { track, replaceTrack: vi.fn(async () => {}) } };
  }
  async createOffer() {
    return {
      type: "offer",
      sdp: "m=audio 9 X 111\r\na=rtpmap:111 opus/48000/2\r\na=fmtp:111 minptime=10\r\na=mid:0\r\n",
    };
  }
  async createAnswer() {
    return { type: "answer", sdp: "answer\r\n" };
  }
  async setLocalDescription(d: RTCSessionDescriptionInit) {
    this.local = d;
    this.localDescription = d;
    this.iceGatheringState = "gathering";
    if (FakePeerConnection.autoGather) queueMicrotask(() => this.finishGathering());
  }
  async setRemoteDescription(d: RTCSessionDescriptionInit) {
    this.remote.push(d);
    if (d.type === "answer" && FakePeerConnection.autoConnect)
      queueMicrotask(() => this.setConnectionState("connected"));
  }
  close() {
    this.closed = true;
  }
}

const noCallbacks = { onRemoteAudio: () => {}, onRemoteGone: () => {}, onFailed: () => {} };

function setup() {
  FakePeerConnection.autoConnect = true;
  FakePeerConnection.autoGather = true;
  vi.stubGlobal("RTCPeerConnection", FakePeerConnection);
}

describe("CfRealtimeAdapter", () => {
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("publishes with DTX, then pulls only other speakers and answers renegotiation", async () => {
    setup();
    const nameOf = (id: string) => {
      const [sid, source] = id.split(":");
      return source === "bridge" ? `${sid}-bridge` : sid;
    };
    const publish = vi.fn(async (_offer: { type: string; sdp: string }) => ({
      sessionDescription: { type: "answer", sdp: "a" },
    }));
    const pull = vi.fn(async (ids: string[]) => ({
      requiresImmediateRenegotiation: true,
      sessionDescription: { type: "offer", sdp: "o" },
      tracks: ids.map((id, i) => ({ mid: String(i + 1), trackName: nameOf(id) })),
    }));
    const answer = vi.fn(async (_answer: { type: string; sdp: string }) => {});
    const api: CfRealtimeApi = { publish, pull, answer };
    const gone: string[] = [];
    const adapter = new CfRealtimeAdapter(
      "me",
      {} as MediaStreamTrack,
      api,
      { ...noCallbacks, onRemoteGone: (s) => gone.push(s) },
      {
        version: 3,
        tracks: [
          { id: "me:direct", session_id: "me", source: "direct" },
          { id: "bob:direct", session_id: "bob", source: "direct" },
        ],
      },
    );
    await adapter.start();
    // 最初の一覧の pull はキューの後ろで動く。
    await vi.waitFor(() => expect(answer).toHaveBeenCalledTimes(1));
    // 候補を集め終えた SDP（DTX 付き）を送る。answer も同じ。
    expect(publish.mock.calls[0][0].sdp).toContain("usedtx=1");
    expect(publish.mock.calls[0][0].sdp).toContain("a=candidate:1");
    expect(answer.mock.calls[0][0].sdp).toContain("a=candidate:1");
    expect(pull).toHaveBeenCalledWith(["bob:direct"]);

    // 古い版は無視、新しい版で消えた人は止める。
    adapter.updateTracks([], 2);
    expect(gone).toEqual([]);
    adapter.updateTracks([{ id: "carol:bridge", session_id: "carol", source: "bridge" }], 4);
    expect(gone).toEqual(["bob"]);
    await vi.waitFor(() => expect(pull).toHaveBeenLastCalledWith(["carol:bridge"]));

    adapter.close();
    expect(FakePeerConnection.last?.closed).toBe(true);
    expect(gone).toContain("carol");
  });

  it("finishes start() only after the connection is up, and fails it otherwise", async () => {
    setup();
    FakePeerConnection.autoConnect = false;
    const api: CfRealtimeApi = {
      publish: async () => ({ sessionDescription: { type: "answer", sdp: "a" } }),
      pull: async () => ({}),
      answer: async () => {},
    };
    const failed = vi.fn();
    const adapter = new CfRealtimeAdapter(
      "me",
      {} as MediaStreamTrack,
      api,
      { ...noCallbacks, onFailed: failed },
      { tracks: [], version: 0 },
    );
    let done = false;
    const started = adapter.start().then(() => (done = true));
    await vi.waitFor(() => expect(FakePeerConnection.last?.remote).toHaveLength(1));
    expect(done).toBe(false);
    FakePeerConnection.last!.setConnectionState("connected");
    await started;
    expect(done).toBe(true);

    // つながらなかったときは start() が reject し、onFailed は呼ばない（呼び出し側の処理が二重にならない）。
    const second = new CfRealtimeAdapter(
      "me",
      {} as MediaStreamTrack,
      api,
      { ...noCallbacks, onFailed: failed },
      { tracks: [], version: 0 },
    );
    const rejected = expect(second.start()).rejects.toThrow("Cloudflare connection failed");
    await vi.waitFor(() => expect(FakePeerConnection.last?.remote).toHaveLength(1));
    FakePeerConnection.last!.setConnectionState("failed");
    await rejected;
    expect(failed).not.toHaveBeenCalled();

    // つながったあとの失敗は onFailed で知らせる。
    FakePeerConnection.autoConnect = true;
    const third = new CfRealtimeAdapter(
      "me",
      {} as MediaStreamTrack,
      api,
      { ...noCallbacks, onFailed: failed },
      { tracks: [], version: 0 },
    );
    await third.start();
    FakePeerConnection.last!.setConnectionState("failed");
    expect(failed).toHaveBeenCalledWith("cloudflare_failed");
  });

  it("fails start() when ICE gathering or the connection times out, or the adapter closes", async () => {
    setup();
    vi.useFakeTimers();
    FakePeerConnection.autoGather = false;
    const publish = vi.fn(async () => ({ sessionDescription: { type: "answer", sdp: "a" } }));
    const api: CfRealtimeApi = { publish, pull: async () => ({}), answer: async () => {} };
    const make = () =>
      new CfRealtimeAdapter("me", {} as MediaStreamTrack, api, noCallbacks, {
        tracks: [],
        version: 0,
      });

    const gathering = expect(make().start()).rejects.toThrow("ICE gathering timed out");
    await vi.advanceTimersByTimeAsync(CF_ICE_GATHERING_TIMEOUT_MS);
    await gathering;
    // 候補を集めきらない SDP は送らない。
    expect(publish).not.toHaveBeenCalled();

    FakePeerConnection.autoGather = true;
    FakePeerConnection.autoConnect = false;
    const connecting = expect(make().start()).rejects.toThrow("connection timed out");
    await vi.advanceTimersByTimeAsync(CF_CONNECT_TIMEOUT_MS);
    await connecting;

    const closing = make();
    const closed = expect(closing.start()).rejects.toThrow("Closed");
    await vi.advanceTimersByTimeAsync(0);
    closing.close();
    await closed;
  });

  it("keeps successful pulls and retries only failed ones", async () => {
    setup();
    const pull = vi.fn(async (ids: string[]) => ({
      tracks: ids.map((id, i) =>
        id.startsWith("bob")
          ? { mid: null, trackName: "bob", errorCode: "not_found" }
          : { mid: String(i + 1), trackName: id.split(":")[0] },
      ),
    }));
    const api: CfRealtimeApi = {
      publish: async () => ({ sessionDescription: { type: "answer", sdp: "a" } }),
      pull,
      answer: async () => {},
    };
    const tracks = [
      { id: "bob:direct", session_id: "bob", source: "direct" as const },
      { id: "carol:direct", session_id: "carol", source: "direct" as const },
    ];
    const adapter = new CfRealtimeAdapter("me", {} as MediaStreamTrack, api, noCallbacks, {
      tracks,
      version: 1,
    });
    await adapter.start();
    await vi.waitFor(() => expect(pull).toHaveBeenCalledWith(["bob:direct", "carol:direct"]));
    // 次の一覧の更新で、失敗した Bob だけを pull し直す。
    await vi.waitFor(() => {
      adapter.updateTracks(tracks, 2);
      expect(pull).toHaveBeenLastCalledWith(["bob:direct"]);
    });
    adapter.close();
  });

  it("does not let a stale pull failure drop a newer registration of the same id", async () => {
    setup();
    let rejectFirst: (err: Error) => void = () => {};
    const pull = vi
      .fn<(ids: string[]) => Promise<CfTracksResponse>>()
      .mockImplementationOnce(() => new Promise((_, reject) => (rejectFirst = reject)))
      .mockImplementation(async (ids) => ({
        tracks: ids.map((id, i) => ({ mid: String(i + 1), trackName: id.split(":")[0] })),
      }));
    const api: CfRealtimeApi = {
      publish: async () => ({ sessionDescription: { type: "answer", sdp: "a" } }),
      pull,
      answer: async () => {},
    };
    const bob = [{ id: "bob:direct", session_id: "bob", source: "direct" as const }];
    const adapter = new CfRealtimeAdapter("me", {} as MediaStreamTrack, api, noCallbacks, {
      tracks: bob,
      version: 1,
    });
    await adapter.start();
    await vi.waitFor(() => expect(pull).toHaveBeenCalledTimes(1));
    // pull を待つあいだに Bob が消えて、また現れた。
    adapter.updateTracks([], 2);
    adapter.updateTracks(bob, 3);
    rejectFirst(new Error("network"));
    // 新しい登録の pull が走る。
    await vi.waitFor(() => expect(pull).toHaveBeenCalledTimes(2));
    expect(pull).toHaveBeenLastCalledWith(["bob:direct"]);
    adapter.close();
  });

  it("publishes the microphone switched before start()", async () => {
    setup();
    const api: CfRealtimeApi = {
      publish: async () => ({ sessionDescription: { type: "answer", sdp: "a" } }),
      pull: async () => ({}),
      answer: async () => {},
    };
    const oldMic = { id: "old" } as MediaStreamTrack;
    const newMic = { id: "new" } as MediaStreamTrack;
    const adapter = new CfRealtimeAdapter("me", oldMic, api, noCallbacks, {
      tracks: [],
      version: 0,
    });
    await adapter.replaceMicTrack(newMic);
    await adapter.start();
    expect(FakePeerConnection.last?.added).toEqual([newMic]);
    adapter.close();
  });
});
