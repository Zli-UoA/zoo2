import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { ApiError, buildAuthHeaders, createConfiguredApiClient } from "../api/client";
import { joinUrl, resolveConfig, type AppConfig } from "../config";
import { useCurrentUser } from "../../hooks/useCurrentUser";
import {
  isActiveCallSummary,
  isCallParticipant,
  type ActiveCallSummary,
  type CallConnectionState,
  type CallContextValue,
  type CallParticipant,
} from "./types";

const defaultCallContext: CallContextValue = {
  currentChannelId: null,
  connectionState: "idle",
  error: null,
  participants: [],
  isMuted: false,
  speakingUserIds: new Set(),
  activeCalls: new Map(),
  joinCall: async () => {},
  leaveCall: async () => {},
  toggleMute: async () => {},
};

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
    if (audioContext.state === "suspended") {
      void audioContext.resume().catch(() => {});
    }

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
  const { currentUserId } = useCurrentUser();
  const config = useMemo(() => explicitConfig ?? resolveConfig(), [explicitConfig]);
  const apiClient = useMemo(() => createConfiguredApiClient(config), [config]);

  const effectiveUserId = currentUserId ?? config.devUserId ?? "me";

  const [currentChannelId, setCurrentChannelId] = useState<string | null>(null);
  const [connectionState, setConnectionState] = useState<CallConnectionState>("idle");
  const [error, setError] = useState<string | null>(null);
  const [participants, setParticipants] = useState<CallParticipant[]>([]);
  const [isMuted, setIsMuted] = useState(false);
  const [speakingUserIds, setSpeakingUserIds] = useState<Set<string>>(new Set());
  const [activeCalls, setActiveCalls] = useState<Map<string, number>>(new Map());

  const currentChannelIdRef = useRef<string | null>(null);
  currentChannelIdRef.current = currentChannelId;

  const participantsRef = useRef<CallParticipant[]>([]);
  participantsRef.current = participants;

  const sessionUserMapRef = useRef<Map<string, string>>(new Map());

  const sessionIdRef = useRef<string | null>(null);
  const localStreamRef = useRef<MediaStream | null>(null);
  const localVolumeCleanupRef = useRef<(() => void) | null>(null);
  const sseAbortRef = useRef<AbortController | null>(null);
  const joinAbortRef = useRef<AbortController | null>(null);
  const isMutedRef = useRef(false);
  isMutedRef.current = isMuted;

  interface PeerEntry {
    pc: RTCPeerConnection;
    userId: string;
    cleanupAudio?: () => void;
    pendingCandidates: RTCIceCandidateInit[];
  }
  const peersRef = useRef<Map<string, PeerEntry>>(new Map());
  const pendingEarlyCandidatesRef = useRef<Map<string, RTCIceCandidateInit[]>>(new Map());
  const iceServersRef = useRef<RTCIceServer[]>([{ urls: "stun:stun.l.google.com:19302" }]);
  const callGenerationRef = useRef(0);

  const sendSignalWithRetry = useCallback(
    async (
      channelId: string,
      mySessionId: string,
      targetSessionId: string,
      signal: unknown,
      retries = 2,
    ) => {
      for (let attempt = 0; attempt <= retries; attempt++) {
        try {
          await apiClient.request(`/api/channels/${channelId}/call/signal`, {
            method: "POST",
            body: {
              session_id: mySessionId,
              target_session_id: targetSessionId,
              signal,
            },
          });
          return;
        } catch (err: unknown) {
          const status =
            err instanceof ApiError ? err.status : (err as { status?: number })?.status;
          if (status === 404 || status === 403 || status === 400 || attempt === retries) {
            throw err;
          }
          await new Promise((resolve) => setTimeout(resolve, 300 * (attempt + 1)));
        }
      }
    },
    [apiClient],
  );

  // アクティブな通話一覧の定期取得 (10秒ごと)
  const pollActiveCalls = useCallback(async () => {
    try {
      const list = await apiClient.request<ActiveCallSummary[]>("/api/calls/active");
      if (!Array.isArray(list)) return;
      const map = new Map<string, number>();
      for (const item of list) {
        if (isActiveCallSummary(item) && item.participant_count > 0) {
          map.set(item.channel_id, item.participant_count);
        }
      }
      setActiveCalls((prev) => {
        if (prev.size !== map.size) return map;
        for (const [k, v] of map.entries()) {
          if (prev.get(k) !== v) return map;
        }
        return prev;
      });
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
    pendingEarlyCandidatesRef.current.clear();
    sessionUserMapRef.current.clear();
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

  // Provider アンマウント時のクリーンアップ
  useEffect(() => {
    return () => {
      joinAbortRef.current?.abort();
      sseAbortRef.current?.abort();
      cleanupPeers();
      closeMedia();
    };
  }, [cleanupPeers, closeMedia]);

  const leaveCall = useCallback(async () => {
    // 世代を進めて古い非同期コールバックや SSE ループを無効化 ([P1])
    callGenerationRef.current++;

    // 進行中の joinCall があれば直ちに中断 ([P1])
    if (joinAbortRef.current) {
      joinAbortRef.current.abort();
      joinAbortRef.current = null;
    }

    const channelId = currentChannelIdRef.current;
    const sessionId = sessionIdRef.current;

    if (sseAbortRef.current) {
      sseAbortRef.current.abort();
      sseAbortRef.current = null;
    }

    // まずローカルのメディアおよび接続を即座に停止 ([P2])
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

    // サーバーへの退出リクエストはメディア停止後に送信（待たされてもマイク停止に影響しない）
    if (channelId && sessionId) {
      try {
        await apiClient.request(`/api/channels/${channelId}/call/leave`, {
          method: "POST",
          body: { session_id: sessionId },
        });
      } catch {}
    }
  }, [apiClient, cleanupPeers, closeMedia, pollActiveCalls]);

  // タブ閉じ・アンロード時の退出処理 (fetch keepalive + 認証ヘッダー)
  useEffect(() => {
    const handleUnload = () => {
      const channelId = currentChannelIdRef.current;
      const sessionId = sessionIdRef.current;
      if (channelId && sessionId) {
        const payload = JSON.stringify({ session_id: sessionId });
        const url = joinUrl(config.apiBaseUrl, `/api/channels/${channelId}/call/leave`);
        void (async () => {
          try {
            const headers = await buildAuthHeaders({ devUserId: config.devUserId });
            headers.set("Content-Type", "application/json");
            void fetch(url, {
              method: "POST",
              headers,
              body: payload,
              keepalive: true,
            });
          } catch {}
        })();
      }
    };

    window.addEventListener("beforeunload", handleUnload);
    window.addEventListener("pagehide", handleUnload);
    return () => {
      window.removeEventListener("beforeunload", handleUnload);
      window.removeEventListener("pagehide", handleUnload);
    };
  }, [config.apiBaseUrl, config.devUserId]);

  const toggleMute = useCallback(async () => {
    const channelId = currentChannelIdRef.current;
    const sessionId = sessionIdRef.current;
    if (!channelId || !sessionId || !localStreamRef.current) return;

    const nextMuted = !isMuted;
    isMutedRef.current = nextMuted;
    for (const track of localStreamRef.current.getAudioTracks()) {
      track.enabled = !nextMuted;
    }
    setIsMuted(nextMuted);

    // 自分の参加者アバターにもミュート状態を即座に反映 ([P3])
    setParticipants((prev) =>
      prev.map((p) => (p.session_id === sessionId ? { ...p, is_muted: nextMuted } : p)),
    );

    try {
      await apiClient.request(`/api/channels/${channelId}/call/state`, {
        method: "POST",
        body: { session_id: sessionId, is_muted: nextMuted },
      });
    } catch {}
  }, [apiClient, isMuted]);

  const updateSpeakingUser = useCallback((userId: string, isSpeaking: boolean) => {
    setSpeakingUserIds((prev) => {
      const next = new Set(prev);
      if (isSpeaking) next.add(userId);
      else next.delete(userId);

      if (prev.size !== next.size) return next;
      for (const id of next) {
        if (!prev.has(id)) return next;
      }
      return prev;
    });
  }, []);

  // 通話中の定期 heartbeat 送信 (10秒ごと、TTL 30秒に対して安全な間隔) ([P1])
  useEffect(() => {
    if (connectionState !== "connected" || !currentChannelId) {
      return;
    }
    const interval = window.setInterval(() => {
      const channelId = currentChannelIdRef.current;
      const sessionId = sessionIdRef.current;
      if (channelId && sessionId) {
        void apiClient
          .request(`/api/channels/${channelId}/call/heartbeat`, {
            method: "POST",
            body: { session_id: sessionId },
          })
          .catch((err: unknown) => {
            const status =
              err instanceof ApiError ? err.status : (err as { status?: number })?.status;
            if (status === 404 || status === 403) {
              void leaveCall();
            }
          });
      }
    }, 10000);

    return () => window.clearInterval(interval);
  }, [apiClient, connectionState, currentChannelId, leaveCall]);

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

      // peer 作成前に届いていた ICE candidate があればマージ ([P2])
      const earlyCandidates = pendingEarlyCandidatesRef.current.get(targetSessionId);
      if (earlyCandidates && earlyCandidates.length > 0) {
        entry.pendingCandidates.push(...earlyCandidates);
        pendingEarlyCandidatesRef.current.delete(targetSessionId);
      }

      peersRef.current.set(targetSessionId, entry);

      if (localStreamRef.current) {
        for (const track of localStreamRef.current.getAudioTracks()) {
          pc.addTrack(track, localStreamRef.current);
        }
      }

      pc.onicecandidate = (event) => {
        if (event.candidate) {
          void sendSignalWithRetry(channelId, mySessionId, targetSessionId, {
            type: "candidate",
            candidate: event.candidate.toJSON(),
          }).catch(() => {});
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
          updateSpeakingUser(targetUserId, speaking);
        });

        entry.cleanupAudio = () => {
          stopMeter();
          updateSpeakingUser(targetUserId, false);
          audio.pause();
          audio.srcObject = null;
        };
      };

      return pc;
    },
    [sendSignalWithRetry, updateSpeakingUser],
  );

  const joinCall = useCallback(
    async (channelId: string) => {
      if (currentChannelIdRef.current === channelId && connectionState === "connected") {
        return;
      }

      if (currentChannelIdRef.current) {
        await leaveCall();
      }

      const currentGen = ++callGenerationRef.current;

      if (joinAbortRef.current) {
        joinAbortRef.current.abort();
      }
      const joinAbort = new AbortController();
      joinAbortRef.current = joinAbort;

      setCurrentChannelId(channelId);
      setConnectionState("connecting");
      setError(null);

      // マイク取得前の既存状態クリーンアップ ([P1])
      cleanupPeers();
      closeMedia();

      // マイク取得
      let stream: MediaStream;
      try {
        stream = await navigator.mediaDevices.getUserMedia({
          audio: { echoCancellation: true, noiseSuppression: true, autoGainControl: true },
          video: false,
        });
      } catch (err: unknown) {
        if (callGenerationRef.current !== currentGen || joinAbort.signal.aborted) return;
        cleanupPeers();
        closeMedia();
        const e = err as Error;
        const msg =
          e.name === "NotAllowedError" || e.name === "PermissionDeniedError"
            ? t("call.micPermissionDenied")
            : e.name === "NotFoundError" || e.name === "DevicesNotFoundError"
              ? t("call.micNotFound")
              : e.message || t("call.micNotFound");

        setError(msg);
        toast.error(msg);
        setConnectionState("error");
        setCurrentChannelId(null);
        sessionIdRef.current = null;
        setParticipants([]);
        return;
      }

      // マイク取得中に世代が変わった場合（退出や別通話への参加）は即座にトラックを停止して中止 ([P1])
      if (callGenerationRef.current !== currentGen || joinAbort.signal.aborted) {
        for (const track of stream.getTracks()) {
          track.stop();
        }
        return;
      }

      localStreamRef.current = stream;
      setIsMuted(false);
      isMutedRef.current = false;

      // 発話検知 (最新の isMutedRef を参照して通話切り替え後も正常に反映 [P3])
      localVolumeCleanupRef.current = createVolumeMeter(stream, (speaking) => {
        updateSpeakingUser(effectiveUserId, speaking && !isMutedRef.current);
      });

      const mySessionId = crypto.randomUUID();
      sessionIdRef.current = mySessionId;

      // SSE 接続
      const abortController = new AbortController();
      sseAbortRef.current = abortController;

      try {
        const authHeaders = await buildAuthHeaders({ devUserId: config.devUserId });
        if (callGenerationRef.current !== currentGen) return;
        authHeaders.set("Accept", "text/event-stream");

        const streamUrl = joinUrl(
          config.apiBaseUrl,
          `/api/channels/${channelId}/call/stream?session_id=${mySessionId}`,
        );
        const response = await fetch(streamUrl, {
          headers: authHeaders,
          signal: abortController.signal,
        });

        if (callGenerationRef.current !== currentGen) return;

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
          if (callGenerationRef.current !== currentGen) return;
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
              const rawParticipants = Array.isArray(data.participants) ? data.participants : [];
              const roomParticipants = rawParticipants.filter(isCallParticipant);
              const ice = (data.ice_servers as RTCIceServer[]) ?? [
                { urls: "stun:stun.l.google.com:19302" },
              ];
              iceServersRef.current = ice;
              setParticipants(roomParticipants);

              for (const p of roomParticipants) {
                sessionUserMapRef.current.set(p.session_id, p.user_id);
              }

              // 接続完了前に設定されたミュート状態をバックエンドに即時反映 ([P2])
              if (isMutedRef.current) {
                void apiClient
                  .request(`/api/channels/${channelId}/call/state`, {
                    method: "POST",
                    body: { session_id: mySessionId, is_muted: true },
                  })
                  .catch(() => {});
              }

              // 同時参加時の Offer 衝突 (Glare) 回避 & 冪等性ガード ([P2]):
              // 既に PeerConnection が存在する場合は重複作成しない。
              // セッション ID の辞書順比較で小さい側のみが Offer を送信する。
              for (const p of roomParticipants) {
                if (p.session_id === mySessionId) continue;
                if (peersRef.current.has(p.session_id)) continue;
                if (mySessionId.localeCompare(p.session_id) < 0) {
                  const pc = createPeer(channelId, mySessionId, p.session_id, p.user_id, ice);
                  try {
                    const offer = await pc.createOffer();
                    await pc.setLocalDescription(offer);
                    await sendSignalWithRetry(channelId, mySessionId, p.session_id, {
                      type: "offer",
                      sdp: offer.sdp,
                    });
                  } catch (offerErr) {
                    console.error("Failed to send offer", offerErr);
                  }
                }
              }
            } else if (eventType === "participant_joined") {
              const newP = isCallParticipant(data.participant) ? data.participant : null;
              if (newP && newP.session_id !== mySessionId) {
                sessionUserMapRef.current.set(newP.session_id, newP.user_id);
                setParticipants((prev) => {
                  if (prev.some((p) => p.session_id === newP.session_id)) return prev;
                  return [...prev, newP];
                });
                void pollActiveCalls();

                // 既にピアが存在する場合は重複作成・送信を防止 ([P2])
                if (peersRef.current.has(newP.session_id)) continue;

                // 自身が Offer 送信側 (mySessionId < newP.session_id) の場合は新規参加者へ Offer を送信 ([P2])
                if (mySessionId.localeCompare(newP.session_id) < 0) {
                  const pc = createPeer(
                    channelId,
                    mySessionId,
                    newP.session_id,
                    newP.user_id,
                    iceServersRef.current,
                  );
                  try {
                    const offer = await pc.createOffer();
                    await pc.setLocalDescription(offer);
                    await sendSignalWithRetry(channelId, mySessionId, newP.session_id, {
                      type: "offer",
                      sdp: offer.sdp,
                    });
                  } catch (offerErr) {
                    console.error("Failed to send offer to new participant", offerErr);
                  }
                }
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
                pendingEarlyCandidatesRef.current.delete(leftSid);
                sessionUserMapRef.current.delete(leftSid);
                setParticipants((prev) => prev.filter((p) => p.session_id !== leftSid));
                void pollActiveCalls();
              }
            } else if (eventType === "participant_updated") {
              const updatedP = isCallParticipant(data.participant) ? data.participant : null;
              if (updatedP) {
                sessionUserMapRef.current.set(updatedP.session_id, updatedP.user_id);
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
                const targetUserId =
                  sessionUserMapRef.current.get(fromSid) ??
                  participantsRef.current.find((p) => p.session_id === fromSid)?.user_id ??
                  fromSid;

                const pc = createPeer(
                  channelId,
                  mySessionId,
                  fromSid,
                  targetUserId,
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

                  await sendSignalWithRetry(channelId, mySessionId, fromSid, {
                    type: "answer",
                    sdp: answer.sdp,
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
                } else {
                  // Peer 作成前 (Offer 受信前) に届いた candidate を早期保留バッファに保存 ([P2])
                  const list = pendingEarlyCandidatesRef.current.get(fromSid) ?? [];
                  list.push(signal.candidate);
                  pendingEarlyCandidatesRef.current.set(fromSid, list);
                }
              }
            }
          }
        }

        // 古い世代のループ終了・後始末が新しい通話のリソースを破壊しないようガード ([P1])
        if (callGenerationRef.current !== currentGen) {
          return;
        }

        // SSE 終了時 (done) の後始末
        cleanupPeers();
        closeMedia();
        sessionIdRef.current = null;
        setCurrentChannelId(null);
        setParticipants([]);

        if (!abortController.signal.aborted) {
          // 意図しない切断の場合: エラーとして通知し、状態を error に設定 ([P2])
          const disconnectMsg = t("call.disconnected");
          setError(disconnectMsg);
          toast.error(disconnectMsg);
          setConnectionState("error");
        } else {
          setConnectionState("idle");
        }
      } catch (err: unknown) {
        if (callGenerationRef.current !== currentGen) {
          return;
        }

        cleanupPeers();
        closeMedia();
        sessionIdRef.current = null;
        setCurrentChannelId(null);
        setParticipants([]);

        if (!abortController.signal.aborted) {
          const e = err as Error;
          const errorMsg = e.message || t("call.connectError");
          setError(errorMsg);
          toast.error(errorMsg);
          setConnectionState("error");
        } else {
          setConnectionState("idle");
        }
      }
    },
    [
      apiClient,
      cleanupPeers,
      closeMedia,
      config.apiBaseUrl,
      config.devUserId,
      createPeer,
      effectiveUserId,
      leaveCall,
      pollActiveCalls,
      sendSignalWithRetry,
      t,
      updateSpeakingUser,
    ],
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
  return context ?? defaultCallContext;
}
