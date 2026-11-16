import { expect, it, vi } from "vite-plus/test";
import { DmMedia } from "./dm-media";
it("negotiates two real audio-only WebRTC peers and releases capture on close", async () => {
  const context = new AudioContext();
  const source = context.createMediaStreamDestination().stream;
  const captures: MediaStream[] = [];
  const acquire = async () => {
    const stream = new MediaStream(source.getAudioTracks().map((track) => track.clone()));
    captures.push(stream);
    return stream;
  };
  const original = window.RTCPeerConnection;
  const peers: RTCPeerConnection[] = [];
  const constructor = vi.spyOn(window, "RTCPeerConnection").mockImplementation(function (config) {
    const pc = new original(config);
    peers.push(pc);
    return pc;
  });
  const connectedA = vi.fn();
  const connectedB = vi.fn();
  const failed = vi.fn();
  const signals: unknown[] = [];
  let a!: DmMedia;
  let b!: DmMedia;
  const route =
    (target: () => DmMedia, from: string, connected: () => void) =>
    (event: string, payload?: unknown) => {
      if (event === "signal") {
        signals.push(payload);
        void target().event("signal", {
          from_session_id: from,
          signal: (payload as { signal: unknown }).signal,
        });
      } else if (event === "connected") connected();
      else if (event === "failed") failed();
    };
  a = new DmMedia(
    "a",
    [],
    route(() => b, "a", connectedA),
    acquire,
  );
  b = new DmMedia(
    "b",
    [],
    route(() => a, "b", connectedB),
    acquire,
  );
  try {
    await Promise.all([a.start(), b.start()]);
    await Promise.all([
      a.event("room_state", { participants: [{ session_id: "a" }, { session_id: "b" }] }),
      b.event("room_state", { participants: [{ session_id: "a" }, { session_id: "b" }] }),
    ]);
    await vi.waitFor(
      () => {
        expect(signals.length).toBeGreaterThanOrEqual(2);
        expect(peers).toHaveLength(2);
        for (const pc of peers) {
          expect(pc.signalingState).toBe("stable");
          expect(pc.remoteDescription?.sdp).toContain("m=audio");
          expect(pc.remoteDescription?.sdp).not.toContain("m=video");
          expect(pc.getReceivers()[0].track.kind).toBe("audio");
        }
      },
      { timeout: 10000 },
    );
    expect(failed).not.toHaveBeenCalled();
    a.mute(true);
    expect(captures[0].getAudioTracks()[0].enabled).toBe(false);
    a.close();
    b.close();
    expect(captures.every((s) => s.getTracks().every((t) => t.readyState === "ended"))).toBe(true);
  } finally {
    a.close();
    b.close();
    source.getTracks().forEach((t) => t.stop());
    constructor.mockRestore();
    await context.close();
  }
});
it("stops a late microphone stream after the call has already been closed", async () => {
  const context = new AudioContext();
  const stream = context.createMediaStreamDestination().stream;
  let finish!: (stream: MediaStream) => void;
  const driver = new DmMedia(
    "a",
    [],
    vi.fn(),
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const start = driver.start();
  driver.close();
  finish(stream);
  await start;
  expect(stream.getTracks().every((t) => t.readyState === "ended")).toBe(true);
  await context.close();
});
it("does not end a group call while another member is still negotiating", async () => {
  const context = new AudioContext();
  const stream = context.createMediaStreamDestination().stream;
  const events = vi.fn();
  const driver = new DmMedia("z", [], events, async () => stream);
  try {
    await driver.start();
    await driver.event("room_state", {
      participants: [{ session_id: "a" }, { session_id: "b" }, { session_id: "z" }],
    });
    await driver.event("participant_left", { session_id: "a" });
    expect(events).not.toHaveBeenCalledWith("ended");
    await driver.event("participant_left", { session_id: "b" });
    expect(events).toHaveBeenCalledWith("ended");
  } finally {
    driver.close();
    await context.close();
  }
});
