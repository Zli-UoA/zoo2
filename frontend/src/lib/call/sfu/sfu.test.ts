import { afterEach, describe, expect, it, vi } from "vite-plus/test";

import {
  CF_CONNECT_TIMEOUT_MS,
  CF_ICE_GATHERING_TIMEOUT_MS,
  CF_PULL_RETRY_BASE_MS,
  CF_PULL_RETRY_MAX_ATTEMPTS,
  CfRealtimeAdapter,
  type CfTracksResponse,
  enableOpusDtx,
  isPullRejectedBeforeApply,
  speakerOfTrackName,
  type CfRealtimeApi,
} from "./cf-realtime-adapter";
import { SpeakerTracks, speakerOf } from "./livekit-adapter";
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

  it("reads the session id before the first dot of a LiveKit identity", () => {
    const sid = "6f1c1a2e-0000-4000-8000-000000000001";
    expect(speakerOf(`${sid}.0123456789abcdef`, "mic")).toBe(sid);
    expect(speakerOf(`${sid}.a.b`, "mic")).toBe(sid);
    // 割り当てが変わって identity が変わっても、同じ話者として扱う。
    expect(speakerOf(`${sid}.1`, "mic")).toBe(speakerOf(`${sid}.2`, "mic"));
  });
});

describe("SpeakerTracks", () => {
  // 中身で区別できるよう、呼ぶたびに別の id を持たせる。
  let streamId = 0;
  const stream = () => ({ id: `stream-${++streamId}` }) as unknown as MediaStream;

  it("keeps the new identity's audio when the old identity's track goes away", () => {
    const onRemoteAudio = vi.fn();
    const onRemoteGone = vi.fn();
    const tracks = new SpeakerTracks({ onRemoteAudio, onRemoteGone });
    const oldStream = stream();
    const newStream = stream();
    tracks.subscribed("TR_old", "sess-1", oldStream);
    tracks.subscribed("TR_new", "sess-1", newStream);
    onRemoteAudio.mockClear();
    // 前の identity が外れて、前のトラックの購読が外れる。
    tracks.unsubscribed("TR_old");
    expect(onRemoteGone).not.toHaveBeenCalled();
    expect(onRemoteAudio).not.toHaveBeenCalled();
    tracks.unsubscribed("TR_new");
    expect(onRemoteGone).toHaveBeenCalledWith("sess-1");
  });

  it("falls back to the remaining track when the current one goes away first", () => {
    const onRemoteAudio = vi.fn();
    const onRemoteGone = vi.fn();
    const tracks = new SpeakerTracks({ onRemoteAudio, onRemoteGone });
    const oldStream = stream();
    tracks.subscribed("TR_old", "sess-1", oldStream);
    tracks.subscribed("TR_new", "sess-1", stream());
    onRemoteAudio.mockClear();
    tracks.unsubscribed("TR_new");
    expect(onRemoteAudio).toHaveBeenCalledWith("sess-1", oldStream);
    expect(onRemoteGone).not.toHaveBeenCalled();
  });

  it("falls back to the latest remaining track, not the oldest", () => {
    const onRemoteAudio = vi.fn();
    const onRemoteGone = vi.fn();
    const tracks = new SpeakerTracks({ onRemoteAudio, onRemoteGone });
    const a1 = stream();
    const a2 = stream();
    const a3 = stream();
    tracks.subscribed("TR_a1", "a", a1);
    tracks.subscribed("TR_a2", "a", a2);
    tracks.subscribed("TR_a3", "a", a3);
    onRemoteAudio.mockClear();
    tracks.unsubscribed("TR_a3");
    expect(onRemoteAudio).toHaveBeenCalledTimes(1);
    expect(onRemoteAudio).toHaveBeenCalledWith("a", a2);
    expect(onRemoteAudio).not.toHaveBeenCalledWith("a", a1);
    expect(onRemoteGone).not.toHaveBeenCalled();
  });

  it("reports every speaker on clear and ignores unknown tracks", () => {
    const onRemoteGone = vi.fn();
    const tracks = new SpeakerTracks({ onRemoteAudio: vi.fn(), onRemoteGone });
    tracks.unsubscribed("TR_unknown");
    tracks.subscribed("TR_a", "a", stream());
    tracks.subscribed("TR_b", "b", stream());
    tracks.subscribed("TR_b2", "b", stream());
    tracks.clear();
    expect(new Set(onRemoteGone.mock.calls.map(([s]) => s))).toEqual(new Set(["a", "b"]));
    expect(onRemoteGone).toHaveBeenCalledTimes(2);
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
  /** addTransceiver した時点のトラックの enabled。 */
  addedEnabled: boolean[] = [];
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
    this.addedEnabled.push(track.enabled);
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

/** backend が Cloudflare に送る前に断った（ApiError と同じく status を持つ）失敗。 */
function rejectedBeforeApply(status = 404) {
  return Object.assign(new Error(`HTTP ${status}`), { status });
}

function setup() {
  FakePeerConnection.autoConnect = true;
  FakePeerConnection.autoGather = true;
  vi.stubGlobal("RTCPeerConnection", FakePeerConnection);
}

describe("CfRealtimeAdapter", () => {
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
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
    rejectFirst(rejectedBeforeApply());
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

  it("pulls a track again when its rev changes, and keeps it while the rev stays", async () => {
    setup();
    const pull = vi.fn(async (ids: string[]) => ({
      tracks: ids.map((id, i) => ({ mid: String(i + 1), trackName: id.split(":")[0] })),
    }));
    const api: CfRealtimeApi = {
      publish: async () => ({ sessionDescription: { type: "answer", sdp: "a" } }),
      pull,
      answer: async () => {},
    };
    const gone: string[] = [];
    const bob = (rev: string) => [
      { id: "bob:direct", session_id: "bob", source: "direct" as const, rev },
    ];
    const adapter = new CfRealtimeAdapter(
      "me",
      {} as MediaStreamTrack,
      api,
      { ...noCallbacks, onRemoteGone: (s) => gone.push(s) },
      { tracks: bob("r1"), version: 1 },
    );
    await adapter.start();
    await vi.waitFor(() => expect(pull).toHaveBeenCalledTimes(1));
    // 同じ rev なら pull し直さない。
    adapter.updateTracks(bob("r1"), 2);
    await Promise.resolve();
    expect(pull).toHaveBeenCalledTimes(1);
    expect(gone).toEqual([]);
    // 同じ id のまま別のセッションで公開し直された。古い購読をやめて、新しい世代を pull する。
    adapter.updateTracks(bob("r2"), 3);
    expect(gone).toEqual(["bob"]);
    await vi.waitFor(() => expect(pull).toHaveBeenCalledTimes(2));
    expect(pull).toHaveBeenLastCalledWith(["bob:direct"]);
    adapter.close();
  });

  it("retries a pull rejected before apply with backoff without waiting for a list update, up to a limit", async () => {
    setup();
    vi.useFakeTimers();
    const pull = vi.fn<(ids: string[]) => Promise<CfTracksResponse>>(async () => {
      throw rejectedBeforeApply(404);
    });
    const api: CfRealtimeApi = {
      publish: async () => ({ sessionDescription: { type: "answer", sdp: "a" } }),
      pull,
      answer: async () => {},
    };
    const adapter = new CfRealtimeAdapter("me", {} as MediaStreamTrack, api, noCallbacks, {
      tracks: [{ id: "bob:direct", session_id: "bob", source: "direct" }],
      version: 1,
    });
    vi.spyOn(console, "error").mockImplementation(() => {});
    await adapter.start();
    await vi.advanceTimersByTimeAsync(0);
    expect(pull).toHaveBeenCalledTimes(1);
    for (let attempt = 0; attempt < CF_PULL_RETRY_MAX_ATTEMPTS; attempt++) {
      await vi.advanceTimersByTimeAsync(CF_PULL_RETRY_BASE_MS * 2 ** attempt);
      expect(pull).toHaveBeenCalledTimes(attempt + 2);
    }
    // 上限に達したら、次の一覧の更新まで待つ。
    await vi.advanceTimersByTimeAsync(
      CF_PULL_RETRY_BASE_MS * 2 ** (CF_PULL_RETRY_MAX_ATTEMPTS + 2),
    );
    expect(pull).toHaveBeenCalledTimes(CF_PULL_RETRY_MAX_ATTEMPTS + 1);
    adapter.close();
  });

  it("recovers a failed pull on the retry", async () => {
    setup();
    vi.useFakeTimers();
    const pull = vi
      .fn<(ids: string[]) => Promise<CfTracksResponse>>()
      .mockRejectedValueOnce(rejectedBeforeApply())
      .mockImplementation(async (ids) => ({
        tracks: ids.map((id, i) => ({ mid: String(i + 1), trackName: id.split(":")[0] })),
      }));
    const api: CfRealtimeApi = {
      publish: async () => ({ sessionDescription: { type: "answer", sdp: "a" } }),
      pull,
      answer: async () => {},
    };
    const adapter = new CfRealtimeAdapter("me", {} as MediaStreamTrack, api, noCallbacks, {
      tracks: [{ id: "bob:direct", session_id: "bob", source: "direct" }],
      version: 1,
    });
    vi.spyOn(console, "error").mockImplementation(() => {});
    await adapter.start();
    await vi.advanceTimersByTimeAsync(0);
    expect(pull).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(CF_PULL_RETRY_BASE_MS);
    expect(pull).toHaveBeenCalledTimes(2);
    // 成功したら、それ以上 pull しない。
    await vi.advanceTimersByTimeAsync(CF_PULL_RETRY_BASE_MS * 16);
    expect(pull).toHaveBeenCalledTimes(2);
    adapter.close();
  });

  it("retries a per-track errorCode on the same session with backoff", async () => {
    setup();
    vi.useFakeTimers();
    const pull = vi
      .fn<(ids: string[]) => Promise<CfTracksResponse>>()
      .mockResolvedValueOnce({ tracks: [{ mid: null, trackName: "bob", errorCode: "not_found" }] })
      .mockImplementation(async (ids) => ({
        tracks: ids.map((id, i) => ({ mid: String(i + 1), trackName: id.split(":")[0] })),
      }));
    const failed = vi.fn();
    const api: CfRealtimeApi = {
      publish: async () => ({ sessionDescription: { type: "answer", sdp: "a" } }),
      pull,
      answer: async () => {},
    };
    const adapter = new CfRealtimeAdapter(
      "me",
      {} as MediaStreamTrack,
      api,
      { ...noCallbacks, onFailed: failed },
      { tracks: [{ id: "bob:direct", session_id: "bob", source: "direct" }], version: 1 },
    );
    await adapter.start();
    await vi.advanceTimersByTimeAsync(0);
    expect(pull).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(CF_PULL_RETRY_BASE_MS);
    expect(pull).toHaveBeenCalledTimes(2);
    expect(pull).toHaveBeenLastCalledWith(["bob:direct"]);
    expect(failed).not.toHaveBeenCalled();
    adapter.close();
  });

  it("fails the connection instead of retrying when a pull's outcome is unknown", async () => {
    setup();
    vi.useFakeTimers();
    // 5xx（backend は Cloudflare の失敗を 502 にする）・時間切れ・status のない失敗は、結果が分からない。
    // 409 は、pull のあいだに割り当てが変わったとき（Cloudflare に送ったあと）にも返るので、同じく扱う。
    for (const err of [rejectedBeforeApply(502), rejectedBeforeApply(409), new Error("network")]) {
      const pull = vi.fn<(ids: string[]) => Promise<CfTracksResponse>>(async () => {
        throw err;
      });
      const failed = vi.fn();
      const api: CfRealtimeApi = {
        publish: async () => ({ sessionDescription: { type: "answer", sdp: "a" } }),
        pull,
        answer: async () => {},
      };
      const bob = [{ id: "bob:direct", session_id: "bob", source: "direct" as const }];
      const adapter = new CfRealtimeAdapter(
        "me",
        {} as MediaStreamTrack,
        api,
        { ...noCallbacks, onFailed: failed },
        { tracks: [], version: 1 },
      );
      vi.spyOn(console, "error").mockImplementation(() => {});
      await adapter.start();
      adapter.updateTracks(bob, 2);
      await vi.advanceTimersByTimeAsync(0);
      expect(pull).toHaveBeenCalledTimes(1);
      expect(failed).toHaveBeenCalledWith("cloudflare_pull_failed");
      // 同じセッションでは、やり直しも一覧の更新でも pull しない。
      await vi.advanceTimersByTimeAsync(
        CF_PULL_RETRY_BASE_MS * 2 ** (CF_PULL_RETRY_MAX_ATTEMPTS + 2),
      );
      adapter.updateTracks(
        [...bob, { id: "carol:direct", session_id: "carol", source: "direct" }],
        3,
      );
      await vi.advanceTimersByTimeAsync(0);
      expect(pull).toHaveBeenCalledTimes(1);
      expect(failed).toHaveBeenCalledTimes(1);
      adapter.close();
    }
  });

  it("fails start() when the initial pull's outcome is unknown", async () => {
    setup();
    FakePeerConnection.autoConnect = false;
    let rejectPull: (err: Error) => void = () => {};
    const pull = vi.fn(() => new Promise<CfTracksResponse>((_, reject) => (rejectPull = reject)));
    const failed = vi.fn();
    const api: CfRealtimeApi = {
      publish: async () => ({ sessionDescription: { type: "answer", sdp: "a" } }),
      pull,
      answer: async () => {},
    };
    const adapter = new CfRealtimeAdapter(
      "me",
      {} as MediaStreamTrack,
      api,
      { ...noCallbacks, onFailed: failed },
      { tracks: [{ id: "bob:direct", session_id: "bob", source: "direct" }], version: 1 },
    );
    vi.spyOn(console, "error").mockImplementation(() => {});
    const started = expect(adapter.start()).rejects.toThrow("timeout");
    await vi.waitFor(() => expect(pull).toHaveBeenCalledTimes(1));
    // つながるのを待たずに start() を失敗させる（onFailed は呼ばない）。
    rejectPull(new Error("timeout"));
    await started;
    expect(failed).not.toHaveBeenCalled();
    expect(pull).toHaveBeenCalledTimes(1);
    adapter.close();
  });

  it("publishes a muted microphone when created muted", async () => {
    setup();
    const api: CfRealtimeApi = {
      publish: async () => ({ sessionDescription: { type: "answer", sdp: "a" } }),
      pull: async () => ({}),
      answer: async () => {},
    };
    const mic = { enabled: true } as MediaStreamTrack;
    const adapter = new CfRealtimeAdapter(
      "me",
      mic,
      api,
      noCallbacks,
      { tracks: [], version: 0 },
      { muted: true },
    );
    await adapter.start();
    expect(FakePeerConnection.last?.addedEnabled).toEqual([false]);
    adapter.close();
  });
});

describe("isPullRejectedBeforeApply", () => {
  it("accepts only statuses the backend returns before calling Cloudflare", () => {
    for (const status of [400, 403, 404])
      expect(isPullRejectedBeforeApply(rejectedBeforeApply(status))).toBe(true);
    // 409 は Cloudflare に送ったあとにも返る（割り当てが変わった）ので、結果が分からないものとする。
    for (const status of [409, 500, 502, 504])
      expect(isPullRejectedBeforeApply(rejectedBeforeApply(status))).toBe(false);
    expect(isPullRejectedBeforeApply(new Error("network"))).toBe(false);
    expect(isPullRejectedBeforeApply(undefined)).toBe(false);
  });
});
