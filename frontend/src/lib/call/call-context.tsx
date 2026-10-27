import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { useTranslation } from "react-i18next";

import { buildAuthHeaders, createConfiguredApiClient } from "../api/client";
import { resolveConfig, type AppConfig } from "../config";
import type {
  ActiveCallSummary,
  CallConnectionState,
  CallContextValue,
  CallParticipant,
} from "./types";

const CallContext = createContext<CallContextValue | null>(null);

function createVolumeMeter(
  stream: MediaStream,
  onVolume: (isSpeaking: boolean) => void,
): () => void {
  try {
    const AudioContextClass =
      window.AudioContext ||
      (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
    if (!AudioContextClass) return () => {};

    const audioContext = new AudioContextClass();
    const source = audioContext.createMediaStreamSource(stream);
    const analyser = audioContext.createAnalyser();
    analyser.fftSize = 256;
    source.connect(analyser);

    const dataArray = new Uint8Array(analyser.frequencyBinCount);
    let active = true;

    const intervalId = window.setInterval(() => {
      if (!active) return;
      analyser.getByteFrequencyData(dataArray);
      let sum = 0;
      for (let i = 0; i < dataArray.length; i++) {
        sum += dataArray[i];
      }
      const avg = sum / dataArray.length;
      onVolume(avg > 18);
    }, 200);

    return () => {
      active = false;
      window.clearInterval(intervalId);
      try {
        source.disconnect();
        analyser.disconnect();
        audioContext.close().catch(() => {});
      } catch {}
    };
  } catch {
    return () => {};
  }
}

export interface CallProviderProps {
  config?: AppConfig;
  children?: ReactNode;
}

export function CallProvider({ config: explicitConfig, children }: CallProviderProps) {
  const { t } = useTranslation();
  const config = explicitConfig ?? resolveConfig();
  const apiClient = createConfiguredApiClient(config);

  const [currentChannelId, setCurrentChannelId] = useState<string | null>(null);
  const [connectionState, setConnectionState] = useState<CallConnectionState>("idle");
  const [error, setError] = useState<string | null>(null);
  const [participants, setParticipants] = useState<CallParticipant[]>([]);
  const [isMuted, setIsMuted] = useState(false);
  const [speakingUserIds, setSpeakingUserIds] = useState<Set<string>>(new Set());
  const [activeCalls, setActiveCalls] = useState<Map<string, number>>(new Map());

  const currentChannelIdRef = useRef<string | null>(null);
  currentChannelIdRef.current = currentChannelId;

  const sessionIdRef = useRef<string | null>(null);
  const localStreamRef = useRef<MediaStream | null>(null);
  const localVolumeCleanupRef = useRef<(() => void) | null>(null);
  const sseAbortRef = useRef<AbortController | null>(null);

  interface PeerEntry {
    pc: RTCPeerConnection;
    userId: string;
    cleanupAudio?: () => void;
    pendingCandidates: RTCIceCandidateInit[];
  }
  const peersRef = useRef<Map<string, PeerEntry>>(new Map());
  const iceServersRef = useRef<RTCIceServer[]>([{ urls: "stun:stun.l.google.com:19302" }]);

  // アクティブな通話一覧の定期取得 (10秒ごと)
  const pollActiveCalls = useCallback(async () => {
    try {
      const list = await apiClient.request<ActiveCallSummary[]>("/api/calls/active");
      const map = new Map<string, number>();
      for (const item of list) {
        if (item.participant_count > 0) {
          map.set(item.channel_id, item.participant_count);
        }
      }
      setActiveCalls(map);
    } catch {}
  }, [apiClient]);

  useEffect(() => {
    void pollActiveCalls();
    const interval = window.setInterval(pollActiveCalls, 10000);
    return () => window.clearInterval(interval);
  }, [pollActiveCalls]);

  const cleanupPeers = useCallback(() => {
    for (const entry of peersRef.current.values()) {
      entry.cleanupAudio?.();
      entry.pc.close();
    }
    peersRef.current.clear();
  }, []);

  const closeMedia = useCallback(() => {
    localVolumeCleanupRef.current?.();
    localVolumeCleanupRef.current = null;

    if (localStreamRef.current) {
      for (const track of localStreamRef.current.getTracks()) {
        track.stop();
      }
      localStreamRef.current = null;
    }
  }, []);

  const leaveCall = useCallback(async () => {
    const channelId = currentChannelIdRef.current;
    const sessionId = sessionIdRef.current;

    if (sseAbortRef.current) {
      sseAbortRef.current.abort();
      sseAbortRef.current = null;
    }

    if (channelId && sessionId) {
      try {
        await apiClient.request(`/api/channels/${channelId}/call/leave`, {
          method: "POST",
          body: { session_id: sessionId },
        });
      } catch {}
    }

    cleanupPeers();
    closeMedia();

    sessionIdRef.current = null;
    setCurrentChannelId(null);
    setConnectionState("idle");
    setError(null);
    setParticipants([]);
    setIsMuted(false);
    setSpeakingUserIds(new Set());
    void pollActiveCalls();
  }, [apiClient, cleanupPeers, closeMedia, pollActiveCalls]);

  // タブ閉じ・アンロード時の退出処理
  useEffect(() => {
    const handleUnload = () => {
      const channelId = currentChannelIdRef.current;
      const sessionId = sessionIdRef.current;
      if (channelId && sessionId) {
        const payload = JSON.stringify({ session_id: sessionId });
        const url = `/api/channels/${channelId}/call/leave`;
        if (navigator.sendBeacon) {
          navigator.sendBeacon(url, new Blob([payload], { type: "application/json" }));
        }
      }
    };

    window.addEventListener("beforeunload", handleUnload);
    window.addEventListener("pagehide", handleUnload);
    return () => {
      window.removeEventListener("beforeunload", handleUnload);
      window.removeEventListener("pagehide", handleUnload);
    };
  }, []);

  const toggleMute = useCallback(async () => {
    const channelId = currentChannelIdRef.current;
    const sessionId = sessionIdRef.current;
    if (!channelId || !sessionId || !localStreamRef.current) return;

    const nextMuted = !isMuted;
    for (const track of localStreamRef.current.getAudioTracks()) {
      track.enabled = !nextMuted;
    }
    setIsMuted(nextMuted);

    try {
      await apiClient.request(`/api/channels/${channelId}/call/state`, {
        method: "POST",
        body: { session_id: sessionId, is_muted: nextMuted },
      });
    } catch {}
  }, [apiClient, isMuted]);

  const createPeer = useCallback(
    (
      channelId: string,
      mySessionId: string,
      targetSessionId: string,
      targetUserId: string,
      iceServers: RTCIceServer[],
    ): RTCPeerConnection => {
      const existing = peersRef.current.get(targetSessionId);
      if (existing) {
        existing.cleanupAudio?.();
        existing.pc.close();
      }

      const pc = new RTCPeerConnection({ iceServers });
      const entry: PeerEntry = {
        pc,
        userId: targetUserId,
        pendingCandidates: [],
      };
      peersRef.current.set(targetSessionId, entry);

      if (localStreamRef.current) {
        for (const track of localStreamRef.current.getAudioTracks()) {
          pc.addTrack(track, localStreamRef.current);
        }
      }

      pc.onicecandidate = (event) => {
        if (event.candidate) {
          void apiClient.request(`/api/channels/${channelId}/call/signal`, {
            method: "POST",
            body: {
              session_id: mySessionId,
              target_session_id: targetSessionId,
              signal: { type: "candidate", candidate: event.candidate.toJSON() },
            },
          });
        }
      };

      pc.ontrack = (event) => {
        const stream = event.streams[0];
        if (!stream) return;

        const audio = new Audio();
        audio.srcObject = stream;
        audio.autoplay = true;
        void audio.play().catch(() => {});

        const stopMeter = createVolumeMeter(stream, (speaking) => {
          setSpeakingUserIds((prev) => {
            const next = new Set(prev);
            if (speaking) next.add(targetUserId);
            else next.delete(targetUserId);
            return next;
          });
        });

        entry.cleanupAudio = () => {
          stopMeter();
          audio.pause();
          audio.srcObject = null;
        };
      };

      return pc;
    },
    [apiClient],
  );

  const joinCall = useCallback(
    async (channelId: string) => {
      if (currentChannelIdRef.current === channelId && connectionState === "connected") {
        return;
      }

      if (currentChannelIdRef.current) {
        await leaveCall();
      }

      setCurrentChannelId(channelId);
      setConnectionState("connecting");
      setError(null);

      // マイク取得
      let stream: MediaStream;
      try {
        stream = await navigator.mediaDevices.getUserMedia({
          audio: { echoCancellation: true, noiseSuppression: true, autoGainControl: true },
          video: false,
        });
      } catch (err: unknown) {
        const e = err as Error;
        const msg =
          e.name === "NotAllowedError" || e.name === "PermissionDeniedError"
            ? t("call.micPermissionDenied")
            : e.name === "NotFoundError" || e.name === "DevicesNotFoundError"
              ? t("call.micNotFound")
              : e.message || t("call.micNotFound");

        setError(msg);
        setConnectionState("error");
        setCurrentChannelId(null);
        return;
      }

      localStreamRef.current = stream;
      setIsMuted(false);

      // 発話検知
      localVolumeCleanupRef.current = createVolumeMeter(stream, (speaking) => {
        setSpeakingUserIds((prev) => {
          const myUser = config.devUserId ?? "me";
          const next = new Set(prev);
          if (speaking && !isMuted) next.add(myUser);
          else next.delete(myUser);
          return next;
        });
      });

      const mySessionId = crypto.randomUUID();
      sessionIdRef.current = mySessionId;

      // SSE 接続
      const abortController = new AbortController();
      sseAbortRef.current = abortController;

      try {
        const authHeaders = await buildAuthHeaders({ devUserId: config.devUserId });
        authHeaders.set("Accept", "text/event-stream");

        const streamUrl = `/api/channels/${channelId}/call/stream?session_id=${mySessionId}`;
        const response = await fetch(streamUrl, {
          headers: authHeaders,
          signal: abortController.signal,
        });

        if (!response.ok) {
          const body = await response.text();
          let errorText = t("call.connectError");
          if (response.status === 409) {
            errorText = t("call.full");
          } else if (response.status === 403) {
            errorText = t("call.archived");
          } else if (response.status === 404) {
            errorText = t("call.notFound");
          } else {
            try {
              const parsed = JSON.parse(body);
              if (parsed?.error) errorText = parsed.error;
            } catch {}
          }
          throw new Error(errorText);
        }

        setConnectionState("connected");
        void pollActiveCalls();

        const reader = response.body?.getReader();
        if (!reader) throw new Error("SSE response body is empty");

        const decoder = new TextDecoder();
        let buffer = "";

        while (true) {
          const { done, value } = await reader.read();
          if (done) break;

          buffer += decoder.decode(value, { stream: true });
          const parts = buffer.split("\n\n");
          buffer = parts.pop() ?? "";

          for (const part of parts) {
            let eventType = "message";
            let dataStr = "";

            for (const line of part.split("\n")) {
              if (line.startsWith("event:")) {
                eventType = line.slice(6).trim();
              } else if (line.startsWith("data:")) {
                dataStr += (dataStr ? "\n" : "") + line.slice(5).trim();
              }
            }

            if (!dataStr) continue;

            let data: Record<string, unknown>;
            try {
              data = JSON.parse(dataStr);
            } catch {
              continue;
            }

            if (eventType === "room_state") {
              const roomParticipants = (data.participants as CallParticipant[]) ?? [];
              const ice = (data.ice_servers as RTCIceServer[]) ?? [
                { urls: "stun:stun.l.google.com:19302" },
              ];
              iceServersRef.current = ice;
              setParticipants(roomParticipants);

              // 既存参加者へ Offer を送信 (自分が新規参加者)
              for (const p of roomParticipants) {
                if (p.session_id === mySessionId) continue;
                const pc = createPeer(channelId, mySessionId, p.session_id, p.user_id, ice);
                try {
                  const offer = await pc.createOffer();
                  await pc.setLocalDescription(offer);
                  await apiClient.request(`/api/channels/${channelId}/call/signal`, {
                    method: "POST",
                    body: {
                      session_id: mySessionId,
                      target_session_id: p.session_id,
                      signal: { type: "offer", sdp: offer.sdp },
                    },
                  });
                } catch (offerErr) {
                  console.error("Failed to send offer", offerErr);
                }
              }
            } else if (eventType === "participant_joined") {
              const newP = data.participant as CallParticipant;
              if (newP && newP.session_id !== mySessionId) {
                setParticipants((prev) => {
                  if (prev.some((p) => p.session_id === newP.session_id)) return prev;
                  return [...prev, newP];
                });
                void pollActiveCalls();
              }
            } else if (eventType === "participant_left") {
              const leftSid = data.session_id as string;
              if (leftSid) {
                const entry = peersRef.current.get(leftSid);
                if (entry) {
                  entry.cleanupAudio?.();
                  entry.pc.close();
                  peersRef.current.delete(leftSid);
                }
                setParticipants((prev) => prev.filter((p) => p.session_id !== leftSid));
                void pollActiveCalls();
              }
            } else if (eventType === "participant_updated") {
              const updatedP = data.participant as CallParticipant;
              if (updatedP) {
                setParticipants((prev) =>
                  prev.map((p) => (p.session_id === updatedP.session_id ? updatedP : p)),
                );
              }
            } else if (eventType === "signal") {
              const fromSid = data.from_session_id as string;
              const signal = data.signal as {
                type: "offer" | "answer" | "candidate";
                sdp?: string;
                candidate?: RTCIceCandidateInit;
              };

              if (!fromSid || !signal) continue;

              if (signal.type === "offer" && signal.sdp) {
                const targetParticipant = participants.find((p) => p.session_id === fromSid);
                const pc = createPeer(
                  channelId,
                  mySessionId,
                  fromSid,
                  targetParticipant?.user_id ?? fromSid,
                  iceServersRef.current,
                );

                try {
                  await pc.setRemoteDescription(
                    new RTCSessionDescription({ type: "offer", sdp: signal.sdp }),
                  );
                  // 保留中の ICE Candidate を適用
                  const entry = peersRef.current.get(fromSid);
                  if (entry) {
                    for (const cand of entry.pendingCandidates) {
                      await pc.addIceCandidate(new RTCIceCandidate(cand));
                    }
                    entry.pendingCandidates = [];
                  }

                  const answer = await pc.createAnswer();
                  await pc.setLocalDescription(answer);

                  await apiClient.request(`/api/channels/${channelId}/call/signal`, {
                    method: "POST",
                    body: {
                      session_id: mySessionId,
                      target_session_id: fromSid,
                      signal: { type: "answer", sdp: answer.sdp },
                    },
                  });
                } catch (err) {
                  console.error("Failed to handle offer", err);
                }
              } else if (signal.type === "answer" && signal.sdp) {
                const entry = peersRef.current.get(fromSid);
                if (entry && entry.pc.signalingState === "have-local-offer") {
                  try {
                    await entry.pc.setRemoteDescription(
                      new RTCSessionDescription({ type: "answer", sdp: signal.sdp }),
                    );
                    for (const cand of entry.pendingCandidates) {
                      await entry.pc.addIceCandidate(new RTCIceCandidate(cand));
                    }
                    entry.pendingCandidates = [];
                  } catch (err) {
                    console.error("Failed to set answer", err);
                  }
                }
              } else if (signal.type === "candidate" && signal.candidate) {
                const entry = peersRef.current.get(fromSid);
                if (entry) {
                  if (entry.pc.remoteDescription) {
                    try {
                      await entry.pc.addIceCandidate(new RTCIceCandidate(signal.candidate));
                    } catch (err) {
                      console.error("Failed to add ice candidate", err);
                    }
                  } else {
                    entry.pendingCandidates.push(signal.candidate);
                  }
                }
              }
            }
          }
        }
      } catch (err: unknown) {
        if (!abortController.signal.aborted) {
          const e = err as Error;
          setError(e.message || t("call.connectError"));
          setConnectionState("error");
          cleanupPeers();
          closeMedia();
          setCurrentChannelId(null);
        }
      }
    },
    [apiClient, config.devUserId, createPeer, isMuted, leaveCall, participants, pollActiveCalls, t],
  );

  return (
    <CallContext.Provider
      value={{
        currentChannelId,
        connectionState,
        error,
        participants,
        isMuted,
        speakingUserIds,
        activeCalls,
        joinCall,
        leaveCall,
        toggleMute,
      }}
    >
      {children}
    </CallContext.Provider>
  );
}

export function useCall(): CallContextValue {
  const context = useContext(CallContext);
  if (!context) {
    throw new Error("useCall must be used within a CallProvider");
  }
  return context;
}
