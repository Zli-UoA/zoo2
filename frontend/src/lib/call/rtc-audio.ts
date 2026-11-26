/** Shared audio-only WebRTC primitives for DM/channel and telephone calls. */
export async function acquireCallMicrophone(
  deviceId = "",
  allowFallback = true,
): Promise<{ stream: MediaStream; fellBack: boolean }> {
  const base = { echoCancellation: true, noiseSuppression: true, autoGainControl: true };
  let fellBack = false;
  if (deviceId) {
    try {
      return {
        stream: await navigator.mediaDevices.getUserMedia({
          audio: { ...base, deviceId: { exact: deviceId } },
          video: false,
        }),
        fellBack,
      };
    } catch (error) {
      if (
        !allowFallback ||
        !["OverconstrainedError", "NotFoundError"].includes((error as Error).name)
      )
        throw error;
      fellBack = true;
    }
  }
  return {
    stream: await navigator.mediaDevices.getUserMedia({ audio: base, video: false }),
    fellBack,
  };
}
export function createAudioPeer(iceServers: RTCIceServer[], relay = false) {
  return new RTCPeerConnection({
    iceServers,
    ...(relay ? { iceTransportPolicy: "relay" as const } : {}),
  });
}
export async function createAudioOffer(pc: RTCPeerConnection) {
  const offer = await pc.createOffer();
  await pc.setLocalDescription(offer);
  return offer;
}
export async function flushAudioCandidates(
  pc: RTCPeerConnection,
  candidates: RTCIceCandidateInit[],
) {
  for (const candidate of candidates.splice(0)) await pc.addIceCandidate(candidate);
}
export async function answerAudioOffer(
  pc: RTCPeerConnection,
  sdp: string,
  candidates: RTCIceCandidateInit[],
) {
  await pc.setRemoteDescription({ type: "offer", sdp });
  await flushAudioCandidates(pc, candidates);
  const answer = await pc.createAnswer();
  await pc.setLocalDescription(answer);
  return answer;
}
export async function applyAudioAnswer(
  pc: RTCPeerConnection,
  sdp: string,
  candidates: RTCIceCandidateInit[],
) {
  await pc.setRemoteDescription({ type: "answer", sdp });
  await flushAudioCandidates(pc, candidates);
}

// close() で閉じた接続の replaceTrack は解決しないまま残ることがあるので、閉じたことを待っている側へ知らせる
const closeWaiters = new WeakMap<RTCPeerConnection, Set<() => void>>();

/** 接続を閉じ、`replaceSenderTrack` で待っている差し替えを解放する。 */
export function closePeerConnection(pc: RTCPeerConnection): void {
  pc.close();
  const waiters = closeWaiters.get(pc);
  closeWaiters.delete(pc);
  for (const release of waiters ?? []) release();
}

/**
 * 送信側のトラックを差し替える。差し替えを待つあいだに `closePeerConnection` で閉じられたら、
 * 待たずに false を返す（差し替えの失敗はそのまま返す）。
 */
export async function replaceSenderTrack(
  pc: RTCPeerConnection,
  sender: RTCRtpSender,
  track: MediaStreamTrack | null,
): Promise<boolean> {
  if (pc.signalingState === "closed") return false;
  let release = () => {};
  const closed = new Promise<false>((resolve) => {
    release = () => resolve(false);
  });
  const waiters = closeWaiters.get(pc) ?? new Set();
  closeWaiters.set(pc, waiters);
  waiters.add(release);
  try {
    return await Promise.race([sender.replaceTrack(track).then(() => true as const), closed]);
  } finally {
    waiters.delete(release);
  }
}
