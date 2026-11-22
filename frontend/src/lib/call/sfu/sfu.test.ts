import { describe, expect, it, vi } from "vite-plus/test";

import {
  CfRealtimeAdapter,
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
  listeners = new Map<string, ((e: unknown) => void)[]>();
  connectionState = "new";
  local: RTCSessionDescriptionInit | null = null;
  remote: RTCSessionDescriptionInit[] = [];
  closed = false;
  constructor() {
    FakePeerConnection.last = this;
  }
  addEventListener(type: string, fn: (e: unknown) => void) {
    this.listeners.set(type, [...(this.listeners.get(type) ?? []), fn]);
  }
  addTransceiver(track: MediaStreamTrack) {
    return { sender: { track, replaceTrack: vi.fn(async () => {}) } };
  }
  async createOffer() {
    return {
      type: "offer",
      sdp: "m=audio 9 X 111\r\na=rtpmap:111 opus/48000/2\r\na=fmtp:111 minptime=10\r\na=mid:0\r\n",
    };
  }
  async createAnswer() {
    return { type: "answer", sdp: "answer" };
  }
  async setLocalDescription(d: RTCSessionDescriptionInit) {
    this.local = d;
  }
  async setRemoteDescription(d: RTCSessionDescriptionInit) {
    this.remote.push(d);
  }
  close() {
    this.closed = true;
  }
}

describe("CfRealtimeAdapter", () => {
  it("publishes with DTX, then pulls only other speakers and answers renegotiation", async () => {
    vi.stubGlobal("RTCPeerConnection", FakePeerConnection);
    const publish = vi.fn(async () => ({ sessionDescription: { type: "answer", sdp: "a" } }));
    const pull = vi.fn(async (ids: string[]) => ({
      requiresImmediateRenegotiation: true,
      sessionDescription: { type: "offer", sdp: "o" },
      tracks: ids.map((id, i) => ({ mid: String(i + 1), trackName: id.split(":")[0] })),
    }));
    const answer = vi.fn(async () => {});
    const api: CfRealtimeApi = { publish, pull, answer };
    const gone: string[] = [];
    const adapter = new CfRealtimeAdapter(
      "me",
      {} as MediaStreamTrack,
      api,
      { onRemoteAudio: () => {}, onRemoteGone: (s) => gone.push(s), onFailed: () => {} },
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
    expect(publish).toHaveBeenCalledWith(
      expect.objectContaining({ sdp: expect.stringContaining("usedtx=1") }),
    );
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
    vi.unstubAllGlobals();
  });
});
