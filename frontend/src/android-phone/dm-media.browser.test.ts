import { expect, it, vi } from "vite-plus/test";
import { page } from "vite-plus/test/browser/context";
import { AudioCallMedia as DmMedia } from "../lib/call/audio-media";
it("negotiates two real audio-only WebRTC peers and releases capture on close", async () => {
  const context = new AudioContext();
  const output = context.createMediaStreamDestination();
  const source = output.stream;
  const oscillator = context.createOscillator();
  const gain = context.createGain();
  gain.gain.value = 0.3;
  oscillator.connect(gain).connect(output);
  oscillator.start();
  const startAudio = document.createElement("button");
  startAudio.textContent = "Start test audio";
  document.body.append(startAudio);
  try {
    const resumed = new Promise<void>((resolve, reject) => {
      startAudio.onclick = () => void context.resume().then(resolve, reject);
    });
    await page.getByRole("button", { name: "Start test audio" }).click();
    await resumed;
  } finally {
    startAudio.remove();
  }
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
    const participants = [
      { session_id: "a", user_id: "alice" },
      { session_id: "b", user_id: "bob" },
    ];
    await Promise.all([
      a.event("room_state", { participants }),
      b.event("room_state", { participants }),
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
    await vi.waitFor(async () => expect(await a.remoteAudioLevel("bob")).toBeGreaterThan(0.05), {
      timeout: 10000,
    });
    expect(await a.remoteAudioLevel("alice")).toBe(0);
    a.mute(true);
    expect(captures[0].getAudioTracks()[0].enabled).toBe(false);
    await vi.waitFor(async () => expect(await b.remoteAudioLevel("alice")).toBeLessThan(0.01));
    expect(await a.remoteAudioLevel("bob")).toBeGreaterThan(0.05);
    a.close();
    expect(await a.remoteAudioLevel("bob")).toBe(0);
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

it("reduces audio bitrate on loss and restores WebRTC defaults after sustained recovery", async () => {
  const context = new AudioContext();
  const stream = context.createMediaStreamDestination().stream;
  const pc = new RTCPeerConnection();
  const constructor = vi.spyOn(window, "RTCPeerConnection").mockImplementation(function () {
    return pc;
  });
  const driver = new DmMedia("a", [], vi.fn(), async () => stream);
  try {
    await driver.start();
    await driver.event("room_state", { participants: [{ session_id: "b" }] });
    const [sender] = pc.getSenders();
    const setParameters = vi.spyOn(sender, "setParameters").mockResolvedValue(undefined);
    let timestamp = 0;
    let fractionLost = 0.03;
    vi.spyOn(sender, "getStats").mockImplementation(async () => {
      const report = {
        type: "remote-inbound-rtp",
        timestamp: ++timestamp,
        fractionLost,
        roundTripTime: 0.1,
      };
      return new Map([["remote", report]]) as RTCStatsReport;
    });

    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    vi.spyOn(pc, "connectionState", "get").mockReturnValue("connected");
    pc.dispatchEvent(new Event("connectionstatechange"));
    await vi.advanceTimersByTimeAsync(0);
    expect(setParameters).not.toHaveBeenCalled();

    fractionLost = 0.1;
    await vi.advanceTimersByTimeAsync(3_000);
    expect(setParameters.mock.lastCall?.[0].encodings[0].maxBitrate).toBe(2 ** 14);

    fractionLost = 0;
    await vi.advanceTimersByTimeAsync(6_000);
    expect(setParameters).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(3_000);
    expect(setParameters).toHaveBeenCalledTimes(2);
    expect(setParameters.mock.lastCall?.[0].encodings[0].maxBitrate).toBeUndefined();
    await vi.advanceTimersByTimeAsync(9_000);
    expect(setParameters).toHaveBeenCalledTimes(2);

    driver.close();
    const samples = timestamp;
    await vi.advanceTimersByTimeAsync(9_000);
    expect(timestamp).toBe(samples);
  } finally {
    driver.close();
    vi.useRealTimers();
    constructor.mockRestore();
    await context.close();
  }
});
