import { describe, expect, it, vi } from "vite-plus/test";

import { closePeerConnection, replaceSenderTrack } from "./rtc-audio";

function fakePeer() {
  const close = vi.fn(() => {
    pc.signalingState = "closed";
  });
  const pc = { signalingState: "stable", close };
  return { pc: pc as unknown as RTCPeerConnection, close };
}

describe("replaceSenderTrack", () => {
  it("returns true once the sender has replaced the track", async () => {
    const { pc } = fakePeer();
    const replaceTrack = vi.fn(async () => {});
    const sender = { replaceTrack } as unknown as RTCRtpSender;
    const track = { id: "mic" } as MediaStreamTrack;
    await expect(replaceSenderTrack(pc, sender, track)).resolves.toBe(true);
    expect(replaceTrack).toHaveBeenCalledWith(track);
  });

  it("releases a replacement that never settles when the connection is closed", async () => {
    const { pc, close } = fakePeer();
    // 閉じた接続の replaceTrack は解決しないことがある
    const sender = { replaceTrack: () => new Promise<void>(() => {}) } as unknown as RTCRtpSender;
    const waiting = replaceSenderTrack(pc, sender, null);
    closePeerConnection(pc);
    expect(close).toHaveBeenCalled();
    await expect(waiting).resolves.toBe(false);
    // 閉じたあとは差し替えを呼ばない
    await expect(replaceSenderTrack(pc, sender, null)).resolves.toBe(false);
  });

  it("passes replacement failures through", async () => {
    const { pc } = fakePeer();
    const sender = {
      replaceTrack: async () => {
        throw new Error("boom");
      },
    } as unknown as RTCRtpSender;
    await expect(replaceSenderTrack(pc, sender, null)).rejects.toThrow("boom");
  });
});
