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
