import { openCallStream, readCallEvents } from "./signaling";
import {
  acquireCallMicrophone as acquireMicStream,
  createAudioPeer,
  createAudioOffer,
  answerAudioOffer,
  applyAudioAnswer,
} from "./rtc-audio";
import { visiblePoll } from "../visiblePoll";
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

import { ApiError, createConfiguredApiClient } from "../api/client";
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
import {
  playCallSound,
  primeCallSounds,
  setCallSoundSink,
  shouldPlayRemoteJoin,
  shouldPlayRemoteLeave,
} from "./call-sounds";
import {
  applySink,
  loadSelectedDevice,
  saveSelectedDevice,
  subscribeSelectedDevices,
} from "./call-devices";
import {
  kindsOfIceServers,
  summarizeStats,
  type CallDiagnostics,
  type PeerStatsSample,
} from "./call-diagnostics";
import { reportWebrtcEvent } from "../telemetry/capture";
import { apiPath } from "../api/path";
import { CfRealtimeAdapter, type CfTracksResponse } from "./sfu/cf-realtime-adapter";
import {
  isSfuJoinResponse,
  isSfuTrackInfo,
  type RoomMode,
  type SfuAdapter,
  type SfuAdapterCallbacks,
} from "./sfu/types";

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
  selectedMicId: "",
  selectedSpeakerId: "",
  setMicrophone: async () => {},
  setSpeaker: async () => {},
  getCallDiagnostics: async () => ({
    collectedAt: Date.now(),
    connectionState: "idle",
    isMuted: false,
    micTrackState: null,
    micLabel: null,
    sseConnected: false,
    lastHeartbeatOkAt: null,
    heartbeatFailures: 0,
    iceServerKinds: [],
    peers: [],
  }),
};

export const CallContext = createContext<CallContextValue | null>(null);

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
  // 効果音の判定用。state の反映 (次のレンダー) を待たず、イベント処理時に同期更新する
  const knownSessionIdsRef = useRef<Set<string>>(new Set());

  const sessionIdRef = useRef<string | null>(null);
  const localStreamRef = useRef<MediaStream | null>(null);
  const localVolumeCleanupRef = useRef<(() => void) | null>(null);
  const sseAbortRef = useRef<AbortController | null>(null);
  const joinAbortRef = useRef<AbortController | null>(null);
  const isMutedRef = useRef(false);
  isMutedRef.current = isMuted;

  const [selectedMicId, setSelectedMicId] = useState(() => loadSelectedDevice("mic"));
  const [selectedSpeakerId, setSelectedSpeakerId] = useState(() => loadSelectedDevice("speaker"));
  const micIdRef = useRef(selectedMicId);
  micIdRef.current = selectedMicId;
  const speakerIdRef = useRef(selectedSpeakerId);
  speakerIdRef.current = selectedSpeakerId;
  // 実際に出力している先 (空文字はシステム既定)。適用に失敗した選択値はここへ戻す
  const activeSpeakerIdRef = useRef("");
  // 選択中の出力先を使えないときに、システム既定 (実際の出力先) へ選択値を戻して通知する
  const fallbackSpeaker = useCallback(
    (failedId: string) => {
      if (speakerIdRef.current !== failedId) return;
      // 復元先が失敗した出力先自身なら、システム既定へ戻す
      const fallback = activeSpeakerIdRef.current === failedId ? "" : activeSpeakerIdRef.current;
      activeSpeakerIdRef.current = fallback;
      speakerIdRef.current = fallback;
      saveSelectedDevice("speaker", fallback);
      setCallSoundSink(fallback);
      toast.error(t("call.speakerSwitchFailed"));
    },
    [t],
  );
  // 出力先の適用は 1 件ずつ実行し、常に最新の選択値 (speakerIdRef) を全参加者へ適用する。
  // 適用に失敗し、より新しい選択も無ければ、実際に出力している先 (activeSpeakerIdRef) へ選択値を戻して通知する
  const speakerApplyQueueRef = useRef<Promise<void>>(Promise.resolve());
  const applySpeakerToAll = useCallback(() => {
    const run = speakerApplyQueueRef.current.then(async () => {
      const target = speakerIdRef.current;
      // 効果音も選択中のスピーカーから鳴らす
      setCallSoundSink(target);
      let ok = true;
      let applied = 0;
      for (const audio of remoteAudioElements()) {
        applied++;
        if (!(await applySink(audio, target))) ok = false;
      }
      if (ok) {
        // 実際に適用できた要素があるときだけ、出力先として記録する
        if (applied > 0) activeSpeakerIdRef.current = target;
        return;
      }
      if (speakerIdRef.current !== target) return;
      fallbackSpeaker(target);
      for (const audio of remoteAudioElements()) {
        await applySink(audio, speakerIdRef.current);
      }
    });
    speakerApplyQueueRef.current = run.catch(() => {});
    return run;
  }, [fallbackSpeaker]);

  // 起動時に保存済みの出力先を効果音へ反映する
  useEffect(() => {
    setCallSoundSink(speakerIdRef.current);
  }, []);
  const heartbeatOkAtRef = useRef<number | null>(null);
  const heartbeatFailuresRef = useRef(0);
  const statsPrevRef = useRef<Map<string, PeerStatsSample>>(new Map());
  // マイク切替中 (置換待ち) の新トラック。ミュート操作の対象に含める
  const pendingMicTrackRef = useRef<MediaStreamTrack | null>(null);

  // 最後に適用/要求したマイク ID。保存イベントとの二重切替を防ぐ
  const appliedMicIdRef = useRef(selectedMicId);
  // 実際に送信へ使われているマイク（切替失敗時に選択値を戻す先）
  const activeMicIdRef = useRef(selectedMicId);

  interface PeerEntry {
    pc: RTCPeerConnection;
    userId: string;
    audio?: HTMLAudioElement;
    cleanupAudio?: () => void;
    pendingCandidates: RTCIceCandidateInit[];
  }
  const peersRef = useRef<Map<string, PeerEntry>>(new Map());
  // SFU モード（docs/calls-sfu.md）。mesh の peersRef とは別に、SFU のアダプタと話者ごとの音声を持つ。
  const roomModeRef = useRef<RoomMode>("mesh");
  const sfuAdapterRef = useRef<SfuAdapter | null>(null);
  const sfuAudioRef = useRef<Map<string, { audio: HTMLAudioElement; cleanup: () => void }>>(
    new Map(),
  );
  const sfuStartSeqRef = useRef(0);
  const latestSfuTracksRef = useRef<{ tracks: unknown[]; version: number } | null>(null);
  // アダプタを作る前（join の応答待ちなど）に届いた sfu_reassign。LiveKit の応答のあとで Cloudflare に移る。
  const sfuReassignPendingRef = useRef(false);
  const remoteAudioElements = () => {
    const list: HTMLAudioElement[] = [];
    for (const entry of peersRef.current.values()) if (entry.audio) list.push(entry.audio);
    for (const entry of sfuAudioRef.current.values()) list.push(entry.audio);
    return list;
  };
  const pendingEarlyCandidatesRef = useRef<Map<string, RTCIceCandidateInit[]>>(new Map());
  const iceServersRef = useRef<RTCIceServer[]>([{ urls: "stun:stun.l.google.com:19302" }]);
  const callGenerationRef = useRef(0);
  const leftSessionIdsRef = useRef<Set<string>>(new Set());
  const cachedAuthHeadersRef = useRef<Headers | null>(null);
  const lastHandledOffersRef = useRef<
    Map<string, { signalId?: string; sdp: string; answerSdp?: string }>
  >(new Map());
  const muteSerialQueueRef = useRef<Promise<void>>(Promise.resolve());

  const sendSignalWithRetry = useCallback(
    async (
      channelId: string,
      mySessionId: string,
      targetSessionId: string,
      signal: unknown,
      retries = 2,
    ) => {
      // signal に signal_id がなければ付与して再試行間でも同一 ID を保持 ([P2])
      const signalPayload =
        typeof signal === "object" && signal !== null
          ? {
              signal_id:
                (signal as { signal_id?: string }).signal_id ??
                (typeof crypto !== "undefined" && crypto.randomUUID
                  ? crypto.randomUUID()
                  : `${Date.now()}-${Math.random()}`),
              ...signal,
            }
          : signal;

      for (let attempt = 0; attempt <= retries; attempt++) {
        try {
          await apiClient.request(apiPath`/api/channels/${channelId}/call/signal`, {
            method: "POST",
            body: {
              session_id: mySessionId,
              target_session_id: targetSessionId,
              signal: signalPayload,
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
  const pollActiveCalls = useCallback(
    async (signal?: AbortSignal) => {
      try {
        const list = await apiClient.request<ActiveCallSummary[]>("/api/calls/active", { signal });
        if (signal?.aborted || !Array.isArray(list)) return;
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
    },
    [apiClient],
  );

  useEffect(() => {
    const polling = visiblePoll(pollActiveCalls, { interval: 10000 });
    return () => polling.dispose();
  }, [pollActiveCalls]);

  const closeSfu = useCallback(() => {
    sfuStartSeqRef.current++;
    const adapter = sfuAdapterRef.current;
    sfuAdapterRef.current = null;
    adapter?.close();
    for (const entry of sfuAudioRef.current.values()) entry.cleanup();
    sfuAudioRef.current.clear();
  }, []);

  const cleanupPeers = useCallback(() => {
    closeSfu();
    roomModeRef.current = "mesh";
    latestSfuTracksRef.current = null;
    sfuReassignPendingRef.current = false;
    for (const entry of peersRef.current.values()) {
      entry.cleanupAudio?.();
      entry.pc.close();
    }
    peersRef.current.clear();
    pendingEarlyCandidatesRef.current.clear();
    sessionUserMapRef.current.clear();
    knownSessionIdsRef.current.clear();
    leftSessionIdsRef.current.clear();
    lastHandledOffersRef.current.clear();
    statsPrevRef.current.clear();
    heartbeatOkAtRef.current = null;
    heartbeatFailuresRef.current = 0;
    muteSerialQueueRef.current = Promise.resolve();
  }, [closeSfu]);

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

  const leaveCall = useCallback(
    async (options?: { advanceGen?: boolean; abortJoin?: boolean; silent?: boolean }) => {
      const advanceGen = options?.advanceGen ?? true;
      const abortJoin = options?.abortJoin ?? true;

      // 世代を進めて古い非同期コールバックや SSE ループを無効化 ([P1])
      if (advanceGen) {
        callGenerationRef.current++;
      }

      // 進行中の joinCall があれば直ちに中断 ([P1])
      if (abortJoin && joinAbortRef.current) {
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

      // 自分の退出音 (ユーザー操作による退出のみ。自動退出・通話切替では鳴らさない)
      if (!options?.silent && channelId && sessionId) {
        playCallSound("leave");
      }

      // サーバーへの退出リクエストはメディア停止後に送信（待たされてもマイク停止や通話切替に影響しないよう非同期発火 [P3]）
      if (channelId && sessionId) {
        void apiClient
          .request(apiPath`/api/channels/${channelId}/call/leave`, {
            method: "POST",
            body: { session_id: sessionId },
          })
          .catch(() => {});
      }
    },
    [apiClient, cleanupPeers, closeMedia, pollActiveCalls],
  );

  // タブ閉じ・アンロード時の退出処理 (fetch keepalive + キャッシュ済み認証ヘッダーによる同期発火 [P2])
  useEffect(() => {
    const handleUnload = () => {
      const channelId = currentChannelIdRef.current;
      const sessionId = sessionIdRef.current;
      if (channelId && sessionId) {
        const payload = JSON.stringify({ session_id: sessionId });
        const url = joinUrl(config.apiBaseUrl, apiPath`/api/channels/${channelId}/call/leave`);
        const headers = cachedAuthHeadersRef.current
          ? new Headers(cachedAuthHeadersRef.current)
          : new Headers();
        headers.set("Content-Type", "application/json");
        try {
          void fetch(url, {
            method: "POST",
            headers,
            body: payload,
            keepalive: true,
          });
        } catch {}
      }
    };

    window.addEventListener("beforeunload", handleUnload);
    window.addEventListener("pagehide", handleUnload);
    return () => {
      window.removeEventListener("beforeunload", handleUnload);
      window.removeEventListener("pagehide", handleUnload);
    };
  }, [config.apiBaseUrl]);

  const syncMuteStateToServer = useCallback(
    (channelId: string, sessionId: string) => {
      // 要求を直列化キューで順次送信し、並列 POST による追い越し・逆転を排除 ([P2])
      const currentPromise = muteSerialQueueRef.current
        .then(async () => {
          const currentChannel = currentChannelIdRef.current;
          const currentSession = sessionIdRef.current;
          if (!currentChannel || !currentSession) return;
          if (currentChannel !== channelId || currentSession !== sessionId) return;
          const targetMuted = isMutedRef.current;
          await apiClient.request(apiPath`/api/channels/${currentChannel}/call/state`, {
            method: "POST",
            body: { session_id: currentSession, is_muted: targetMuted },
          });
        })
        .catch((err) => {
          console.error("Failed to sync mute state to server", err);
        });

      muteSerialQueueRef.current = currentPromise;
      return currentPromise;
    },
    [apiClient],
  );

  const toggleMute = useCallback(async () => {
    const channelId = currentChannelIdRef.current;
    const sessionId = sessionIdRef.current;
    if (!channelId || !sessionId || !localStreamRef.current) return;

    const nextMuted = !isMutedRef.current;
    isMutedRef.current = nextMuted;
    for (const track of localStreamRef.current.getAudioTracks()) {
      track.enabled = !nextMuted;
    }
    if (pendingMicTrackRef.current) pendingMicTrackRef.current.enabled = !nextMuted;
    void sfuAdapterRef.current?.setMuted(nextMuted).catch(() => {});
    setIsMuted(nextMuted);
    playCallSound(nextMuted ? "mute" : "unmute");

    // 自分の参加者アバターにもミュート状態を即座に反映 ([P3])
    setParticipants((prev) =>
      prev.map((p) => (p.session_id === sessionId ? { ...p, is_muted: nextMuted } : p)),
    );

    // 要求を直列化キューで順次送信し、並列 POST による追い越し・逆転を排除 ([P2])
    await syncMuteStateToServer(channelId, sessionId);
  }, [syncMuteStateToServer]);

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
      const sentGen = callGenerationRef.current;
      if (channelId && sessionId) {
        void apiClient
          .request(apiPath`/api/channels/${channelId}/call/heartbeat`, {
            method: "POST",
            body: { session_id: sessionId },
          })
          .then(() => {
            if (callGenerationRef.current !== sentGen) return;
            heartbeatOkAtRef.current = Date.now();
            heartbeatFailuresRef.current = 0;
          })
          .catch((err: unknown) => {
            if (callGenerationRef.current === sentGen) {
              heartbeatFailuresRef.current += 1;
              if (heartbeatFailuresRef.current === 3) {
                reportWebrtcEvent({ state: "heartbeat_timeout" });
              }
            }
            // 送信時の世代・セッション・チャンネルが現在の通話と一致する場合のみ退出処理を実行 ([P1])
            if (
              callGenerationRef.current !== sentGen ||
              sessionIdRef.current !== sessionId ||
              currentChannelIdRef.current !== channelId
            ) {
              return;
            }
            const status =
              err instanceof ApiError ? err.status : (err as { status?: number })?.status;
            if (status === 404 || status === 403) {
              void leaveCall({ silent: true });
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

      const pc = createAudioPeer(iceServers);
      pc.addEventListener("iceconnectionstatechange", () => {
        if (pc.iceConnectionState === "failed") {
          reportWebrtcEvent({ state: "ice_failed" });
        }
      });
      pc.addEventListener("connectionstatechange", () => {
        if (pc.connectionState === "failed") {
          reportWebrtcEvent({ state: "connection_failed" });
        }
      });
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
        entry.audio = audio;
        // 保存済みの出力先を適用してから再生を始め、既定スピーカーから漏れないようにする
        const startPlayback = () => {
          if (entry.audio !== audio) return;
          audio.autoplay = true;
          void audio.play().catch(() => {});
        };
        if (speakerIdRef.current) {
          // 出力先の適用は通常の変更と同じ直列キューに入れる。
          // 適用に失敗した場合のフォールバック・既存参加者への反映・選択値の再確認もキュー側で行う
          void applySpeakerToAll().then(startPlayback);
        } else {
          startPlayback();
        }

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
    [applySpeakerToAll, sendSignalWithRetry, updateSpeakingUser],
  );

  const switchMicrophone = useCallback(
    async (deviceId: string, expectedGen: number): Promise<boolean> => {
      // 直列化されたキューの中で、直前の切替が確定した後の最新ストリームを取得する
      const oldStream = localStreamRef.current;
      if (!oldStream || !sessionIdRef.current) return true;
      const gen = callGenerationRef.current;
      // 予約時と異なる通話（退出・再参加を挟んだ場合）への要求は破棄する
      if (gen !== expectedGen) return true;
      let newStream: MediaStream;
      try {
        newStream = (await acquireMicStream(deviceId, false)).stream;
      } catch {
        // 取得中に退出・再参加した場合は、旧通話の失敗として扱わない
        if (callGenerationRef.current !== gen) return true;
        toast.error(t("call.micSwitchFailed"));
        return false;
      }
      // 取得中に退出・切替があった場合は破棄
      if (callGenerationRef.current !== gen || localStreamRef.current !== oldStream) {
        for (const track of newStream.getTracks()) track.stop();
        return true;
      }
      const newTrack = newStream.getAudioTracks()[0];
      if (!newTrack) {
        for (const track of newStream.getTracks()) track.stop();
        return false;
      }
      const stopNewStream = () => {
        for (const track of newStream.getTracks()) track.stop();
      };
      const oldTrack = oldStream.getAudioTracks()[0] ?? null;
      // 送信側へ渡す前にミュート状態を設定し、置換待ち中のミュート操作も新トラックへ反映する
      newTrack.enabled = !isMutedRef.current;
      pendingMicTrackRef.current = newTrack;
      const replaced: RTCRtpSender[] = [];
      const isStale = () =>
        callGenerationRef.current !== gen || localStreamRef.current !== oldStream;
      const clearPending = () => {
        if (pendingMicTrackRef.current === newTrack) pendingMicTrackRef.current = null;
      };
      // 待機中に同じ通話へ追加されたピアも取りこぼさないよう、未置換の送信側が無くなるまで走査し直す。
      // 世代が変われば isStale() で中止するので、別通話のピアには触れない
      const done = new Set<RTCRtpSender>();
      const nextSender = () => {
        for (const entry of peersRef.current.values()) {
          const sender = entry.pc.getSenders().find((s) => s.track?.kind === "audio");
          if (sender && !done.has(sender)) return sender;
        }
        return null;
      };
      // SFU のアダプタも、送信側の 1 つとして差し替える。mesh の差し替えを待つあいだに作られたアダプタも
      // 対象にするため、差し替える直前に読む（それより後に作られたものは startSfu が今のマイクにそろえる）。
      let sfuAdapter: SfuAdapter | null = null;
      let sfuReplaced = false;
      const revertSfu = async () => {
        if (sfuReplaced && sfuAdapter && oldTrack) {
          await sfuAdapter.replaceMicTrack(oldTrack).catch(() => {});
        }
      };
      try {
        for (let sender = nextSender(); sender && !isStale(); sender = nextSender()) {
          done.add(sender);
          await sender.replaceTrack(newTrack);
          replaced.push(sender);
        }
        sfuAdapter = isStale() ? null : sfuAdapterRef.current;
        if (sfuAdapter) {
          await sfuAdapter.replaceMicTrack(newTrack);
          sfuReplaced = true;
        }
      } catch (err) {
        console.error("Failed to replace mic track", err);
        // 置換済みの送信側を旧トラックへ戻し、旧マイクを維持する
        await Promise.all(replaced.map((sender) => sender.replaceTrack(oldTrack).catch(() => {})));
        await revertSfu();
        clearPending();
        stopNewStream();
        if (isStale()) return true;
        toast.error(t("call.micSwitchFailed"));
        return false;
      }
      clearPending();
      // 置換待ちの間に退出・通話切替があった場合は、新しいストリームを破棄する
      if (isStale()) {
        await Promise.all(replaced.map((sender) => sender.replaceTrack(oldTrack).catch(() => {})));
        await revertSfu();
        stopNewStream();
        return true;
      }
      // 置換待ちの間のミュート操作を取りこぼさないよう、切替直前に最新状態を反映する
      newTrack.enabled = !isMutedRef.current;
      localVolumeCleanupRef.current?.();
      localStreamRef.current = newStream;
      localVolumeCleanupRef.current = createVolumeMeter(newStream, (speaking) => {
        updateSpeakingUser(effectiveUserId, speaking && !isMutedRef.current);
      });
      for (const track of oldStream.getTracks()) track.stop();
      activeMicIdRef.current = deviceId;
      return true;
    },
    [effectiveUserId, t, updateSpeakingUser],
  );

  // 連続した切替が同じ旧ストリームを参照して競合しないよう、1 件ずつ実行する
  const micSwitchQueueRef = useRef<Promise<unknown>>(Promise.resolve());
  const micRequestSeqRef = useRef(0);
  // 切替を予約する。失敗し、より新しい選択も無ければ、使用中のマイクへ表示と保存値を戻す
  const enqueueMicSwitch = useCallback(
    (deviceId: string, gen: number) => {
      // 要求ごとの連番で「より新しい要求が無いか」を判定する（A→B→A のような同一 ID の再選択でも食い違わない）
      const seq = ++micRequestSeqRef.current;
      const run = micSwitchQueueRef.current.then(async () => {
        const ok = await switchMicrophone(deviceId, gen);
        if (!ok && micRequestSeqRef.current === seq && callGenerationRef.current === gen) {
          appliedMicIdRef.current = activeMicIdRef.current;
          saveSelectedDevice("mic", activeMicIdRef.current);
        }
      });
      micSwitchQueueRef.current = run.catch(() => {});
      return run;
    },
    [switchMicrophone],
  );
  const setMicrophone = useCallback(
    (deviceId: string) => {
      appliedMicIdRef.current = deviceId;
      saveSelectedDevice("mic", deviceId);
      return enqueueMicSwitch(deviceId, callGenerationRef.current);
    },
    [enqueueMicSwitch],
  );

  const setSpeaker = useCallback(
    async (deviceId: string) => {
      speakerIdRef.current = deviceId;
      saveSelectedDevice("speaker", deviceId);
      await applySpeakerToAll();
    },
    [applySpeakerToAll],
  );

  // 設定画面や他タブでの変更を反映 (通話中なら送信マイクと再生中の出力先にも適用する)
  useEffect(
    () =>
      subscribeSelectedDevices(() => {
        const micId = loadSelectedDevice("mic");
        const speakerId = loadSelectedDevice("speaker");
        setSelectedMicId(micId);
        setSelectedSpeakerId(speakerId);
        if (micId !== appliedMicIdRef.current) {
          appliedMicIdRef.current = micId;
          // 通話外・切断後の選択も「より新しい要求」として扱い、進行中の切替失敗で上書きされないようにする
          micRequestSeqRef.current++;
          if (localStreamRef.current && sessionIdRef.current) {
            void enqueueMicSwitch(micId, callGenerationRef.current);
          }
        }
        if (speakerId !== speakerIdRef.current) {
          speakerIdRef.current = speakerId;
          void applySpeakerToAll();
        }
      }),
    [applySpeakerToAll, enqueueMicSwitch],
  );

  const getCallDiagnostics = useCallback(async (): Promise<CallDiagnostics> => {
    const now = Date.now();
    const track = localStreamRef.current?.getAudioTracks()[0] ?? null;
    const sfuPeers: [string, { pc: RTCPeerConnection; userId: string }][] = (
      sfuAdapterRef.current?.peerConnections() ?? []
    ).map((pc, i) => [`sfu-${sfuAdapterRef.current?.kind}-${i}`, { pc, userId: "sfu" }]);
    const peers = await Promise.all(
      [...peersRef.current.entries(), ...sfuPeers].map(async ([sessionId, entry]) => {
        let summary: ReturnType<typeof summarizeStats> | null = null;
        try {
          const report = await entry.pc.getStats();
          const items: Record<string, unknown>[] = [];
          report.forEach((v) => items.push(v as Record<string, unknown>));
          summary = summarizeStats(items, now, statsPrevRef.current.get(sessionId));
          statsPrevRef.current.set(sessionId, summary.sample);
        } catch {}
        return {
          sessionId,
          userId: entry.userId,
          connectionState: entry.pc.connectionState,
          iceConnectionState: entry.pc.iceConnectionState,
          rttMs: summary?.rttMs ?? null,
          lossRatio: summary?.lossRatio ?? null,
          jitterMs: summary?.jitterMs ?? null,
          inboundKbps: summary?.inboundKbps ?? null,
          outboundKbps: summary?.outboundKbps ?? null,
          candidateType: summary?.candidateType ?? null,
          sample: summary?.sample ?? { at: now, bytesReceived: 0, bytesSent: 0 },
        };
      }),
    );
    return {
      collectedAt: now,
      connectionState,
      isMuted,
      micTrackState: track ? (track.enabled ? track.readyState : "disabled") : null,
      micLabel: track?.label || null,
      sseConnected: connectionState === "connected",
      lastHeartbeatOkAt: heartbeatOkAtRef.current,
      heartbeatFailures: heartbeatFailuresRef.current,
      iceServerKinds: kindsOfIceServers(iceServersRef.current),
      peers,
    };
  }, [connectionState, isMuted]);

  // ---- SFU（docs/calls-sfu.md） ----

  const detachSfuAudio = useCallback((speakerSessionId: string) => {
    const entry = sfuAudioRef.current.get(speakerSessionId);
    if (!entry) return;
    sfuAudioRef.current.delete(speakerSessionId);
    entry.cleanup();
  }, []);

  const attachSfuAudio = useCallback(
    (speakerSessionId: string, stream: MediaStream) => {
      detachSfuAudio(speakerSessionId);
      const userId = sessionUserMapRef.current.get(speakerSessionId) ?? speakerSessionId;
      const audio = new Audio();
      audio.srcObject = stream;
      const startPlayback = () => {
        if (sfuAudioRef.current.get(speakerSessionId)?.audio !== audio) return;
        audio.autoplay = true;
        void audio.play().catch(() => {});
      };
      const stopMeter = createVolumeMeter(stream, (speaking) => {
        updateSpeakingUser(userId, speaking);
      });
      sfuAudioRef.current.set(speakerSessionId, {
        audio,
        cleanup: () => {
          stopMeter();
          updateSpeakingUser(userId, false);
          audio.pause();
          audio.srcObject = null;
        },
      });
      if (speakerIdRef.current) void applySpeakerToAll().then(startPlayback);
      else startPlayback();
    },
    [applySpeakerToAll, detachSfuAudio, updateSpeakingUser],
  );

  /** mesh の接続をすべて閉じる（SFU に移ったあと）。 */
  const closeMeshPeers = useCallback(() => {
    for (const entry of peersRef.current.values()) {
      entry.cleanupAudio?.();
      entry.pc.close();
    }
    peersRef.current.clear();
    pendingEarlyCandidatesRef.current.clear();
    lastHandledOffersRef.current.clear();
  }, []);

  const startSfuRef = useRef<
    (channelId: string, mySessionId: string, gen: number, fallback: boolean) => Promise<void>
  >(async () => {});

  /** SFU に割り当ててもらい、つなぐ。LiveKit につながらなければ Cloudflare でやり直す。 */
  const startSfu = useCallback(
    async (channelId: string, mySessionId: string, gen: number, fallback: boolean) => {
      // 前のアダプタを閉じる（話者の音声も止まる）。
      closeSfu();
      const seq = sfuStartSeqRef.current;
      const isStale = () =>
        callGenerationRef.current !== gen ||
        sfuStartSeqRef.current !== seq ||
        sessionIdRef.current !== mySessionId;
      // マイクは切り替わることがあるので、使う直前に今のものを読む。
      const currentMic = () => localStreamRef.current?.getAudioTracks()[0];
      // SFU につながらないまま（mesh も張らない）にしないよう、通話を終えて参加し直せる状態にする。
      const failSfu = (message: string) => {
        if (isStale()) return;
        void leaveCall({ silent: true });
        setError(message);
        toast.error(message);
        setConnectionState("error");
      };
      if (!currentMic()) return;

      let joined: unknown;
      try {
        joined = await apiClient.request(apiPath`/api/channels/${channelId}/call/sfu/join`, {
          method: "POST",
          body: { session_id: mySessionId, fallback },
        });
      } catch (err) {
        if (isStale()) return;
        console.error("Failed to join the call SFU", err);
        reportWebrtcEvent({ state: "sfu_join_failed" });
        failSfu(t("call.connectError"));
        return;
      }
      if (isStale()) return;
      if (!isSfuJoinResponse(joined)) {
        reportWebrtcEvent({ state: "sfu_join_failed" });
        failSfu(t("call.connectError"));
        return;
      }
      const fallbackToCloudflare = () => {
        reportWebrtcEvent({ state: "sfu_fallback" });
        void startSfuRef.current(channelId, mySessionId, gen, true);
      };

      const callbacks: SfuAdapterCallbacks = {
        onRemoteAudio: (sid, stream) => {
          if (!isStale()) attachSfuAudio(sid, stream);
        },
        onRemoteGone: (sid) => detachSfuAudio(sid),
        onFailed: (reason) => {
          if (isStale()) return;
          console.warn("Call SFU connection failed", reason);
          if (sfuAdapterRef.current?.kind === "livekit") {
            fallbackToCloudflare();
          } else {
            reportWebrtcEvent({ state: "sfu_failed" });
            failSfu(t("call.disconnected"));
          }
        },
      };
      const cfApi = {
        publish: (offer: { type: string; sdp: string }) =>
          apiClient.request<CfTracksResponse>(
            apiPath`/api/channels/${channelId}/call/sfu/cf/publish`,
            { method: "POST", body: { session_id: mySessionId, offer } },
          ),
        pull: (trackIds: string[]) =>
          apiClient.request<CfTracksResponse>(
            apiPath`/api/channels/${channelId}/call/sfu/cf/pull`,
            {
              method: "POST",
              body: { session_id: mySessionId, track_ids: trackIds },
            },
          ),
        answer: async (answer: { type: string; sdp: string }) => {
          await apiClient.request(apiPath`/api/channels/${channelId}/call/sfu/cf/answer`, {
            method: "POST",
            body: { session_id: mySessionId, answer },
          });
        },
      };
      // join の応答と、そのあとに SSE で届いた一覧のうち、新しい方を使う。
      const latest = latestSfuTracksRef.current;
      const initial =
        latest && latest.version > joined.version
          ? { tracks: latest.tracks.filter(isSfuTrackInfo), version: latest.version }
          : { tracks: joined.tracks, version: joined.version };
      let adapter: SfuAdapter;
      let createdMic: MediaStreamTrack;
      if (joined.sfu === "livekit") {
        // livekit-client は大きいので、LiveKit に割り当てられたときだけ読み込む。
        // 読み込めなければ、LiveKit につながらなかったときと同じく Cloudflare でやり直す。
        let LiveKitAdapter: typeof import("./sfu/livekit-adapter").LiveKitAdapter;
        try {
          ({ LiveKitAdapter } = await import("./sfu/livekit-adapter"));
        } catch (err) {
          if (isStale()) return;
          console.error("Failed to load the LiveKit adapter", err);
          fallbackToCloudflare();
          return;
        }
        if (isStale()) return;
        const mic = currentMic();
        if (!mic) return;
        createdMic = mic;
        adapter = new LiveKitAdapter(joined.url, joined.token, mic, callbacks);
      } else {
        const mic = currentMic();
        if (!mic) return;
        createdMic = mic;
        adapter = new CfRealtimeAdapter(mySessionId, mic, cfApi, callbacks, initial);
      }
      sfuAdapterRef.current = adapter;
      if (joined.sfu === "livekit" && sfuReassignPendingRef.current) {
        // 応答を待つあいだに sfu_reassign が届いていた。Cloudflare で join し直す。
        sfuReassignPendingRef.current = false;
        void startSfuRef.current(channelId, mySessionId, gen, false);
        return;
      }
      if (joined.sfu === "cloudflare") sfuReassignPendingRef.current = false;
      try {
        // Cloudflare は ICE・DTLS がつながるまで待つ（LiveKit の connect も同じ）。
        await adapter.start();
      } catch (err) {
        if (isStale()) return;
        console.error("Failed to connect to the call SFU", err);
        if (adapter.kind === "livekit") {
          fallbackToCloudflare();
        } else {
          reportWebrtcEvent({ state: "sfu_failed" });
          failSfu(t("call.connectError"));
        }
        return;
      }
      if (isStale()) return;
      // 生成から接続までのあいだにマイクが切り替わっていたら、今のマイクにそろえる。
      const mic = currentMic();
      if (mic && mic !== createdMic) await adapter.replaceMicTrack(mic).catch(() => {});
      if (isStale()) return;
      if (isMutedRef.current) await adapter.setMuted(true).catch(() => {});
      if (isStale()) return;
      // SFU で送れるようになったので、mesh を閉じる。
      closeMeshPeers();
    },
    [apiClient, attachSfuAudio, closeMeshPeers, closeSfu, detachSfuAudio, leaveCall, t],
  );
  startSfuRef.current = startSfu;

  const joinCall = useCallback(
    async (channelId: string) => {
      if (currentChannelIdRef.current === channelId && connectionState === "connected") {
        return;
      }

      // 進行中の joinCall があれば直ちに中断 ([P1])
      if (joinAbortRef.current) {
        joinAbortRef.current.abort();
        joinAbortRef.current = null;
      }

      // 効果音の AudioContext をユーザー操作の中で有効化
      primeCallSounds();

      // 新しい通話参加要求の世代を即座に確定 ([P1])
      const currentGen = ++callGenerationRef.current;
      const joinAbort = new AbortController();
      joinAbortRef.current = joinAbort;

      if (currentChannelIdRef.current) {
        await leaveCall({ advanceGen: false, abortJoin: false, silent: true });
      }

      // leaveCall 待ちの間に新しい通話切替が要求されていた場合は中断 ([P1])
      if (callGenerationRef.current !== currentGen || joinAbort.signal.aborted) {
        return;
      }

      setCurrentChannelId(channelId);
      setConnectionState("connecting");
      setError(null);

      // マイク取得前の既存状態クリーンアップ ([P1])
      cleanupPeers();
      closeMedia();

      // マイク取得
      let stream: MediaStream;
      let usedMicId = "";
      let micFellBack = false;
      try {
        usedMicId = appliedMicIdRef.current;
        activeMicIdRef.current = usedMicId;
        const acquired = await acquireMicStream(usedMicId, true);
        stream = acquired.stream;
        micFellBack = acquired.fellBack;
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

      // 指定したマイクを使えず既定で取得した場合は、選択値・保存値・使用中の記録を既定へ揃えて通知する
      if (micFellBack && usedMicId) {
        activeMicIdRef.current = "";
        if (appliedMicIdRef.current === usedMicId) {
          appliedMicIdRef.current = "";
          saveSelectedDevice("mic", "");
          usedMicId = "";
        }
        toast.error(t("call.micFallbackToDefault"));
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

      // マイク取得待ちの間に選択が変わっていた場合は、最新の選択へ切り替える
      if (appliedMicIdRef.current !== usedMicId) {
        const latestMicId = appliedMicIdRef.current;
        void enqueueMicSwitch(latestMicId, currentGen);
      }

      // SSE 接続
      const abortController = new AbortController();
      sseAbortRef.current = abortController;

      try {
        const response = await openCallStream(
          config,
          `${apiPath`/api/channels/${channelId}/call/stream?session_id=${mySessionId}`}&sfu=1`,
          {
            signal: abortController.signal,
            onAuth: (headers) => {
              if (callGenerationRef.current !== currentGen) abortController.abort();
              else cachedAuthHeadersRef.current = headers;
            },
          },
        );

        if (callGenerationRef.current !== currentGen) return;

        if (!response.ok) {
          const body = await response.text();
          let errorText = t("call.connectError");
          if (response.status === 409 && body.includes("CALL_SFU_UNSUPPORTED")) {
            errorText = t("call.connectError");
          } else if (response.status === 409) {
            errorText = t("call.full");
          } else if (response.status === 403) {
            errorText = t("call.archived");
          } else if (response.status === 404) {
            errorText = t("call.notFound");
          } else if (response.status === 503) {
            errorText = t("call.reconnecting");
          } else {
            try {
              const parsed = JSON.parse(body);
              if (parsed?.error) errorText = parsed.error;
            } catch {}
          }
          throw new Error(errorText);
        }

        setConnectionState("connected");
        playCallSound("join");
        void pollActiveCalls();

        let hasReceivedRoomState = false;
        interface PendingCallEvent {
          eventType: string;
          data: Record<string, unknown>;
        }
        const pendingEarlyEvents: PendingCallEvent[] = [];

        const handleNonRoomStateEvent = async (
          eventType: string,
          data: Record<string, unknown>,
        ) => {
          if (callGenerationRef.current !== currentGen) return;

          if (eventType === "mode_changed") {
            if (data.mode === "sfu" && roomModeRef.current !== "sfu") {
              roomModeRef.current = "sfu";
              void startSfu(channelId, mySessionId, currentGen, false);
            }
            return;
          }
          if (eventType === "sfu_tracks") {
            const version = typeof data.version === "number" ? data.version : -1;
            const tracks = Array.isArray(data.tracks) ? data.tracks.filter(isSfuTrackInfo) : [];
            const latest = latestSfuTracksRef.current;
            if (!latest || version > latest.version) {
              latestSfuTracksRef.current = { tracks, version };
            }
            sfuAdapterRef.current?.updateTracks(tracks, version);
            return;
          }
          if (eventType === "sfu_reassign") {
            // bridge がないので、LiveKit から Cloudflare に移る。
            const adapter = sfuAdapterRef.current;
            if (adapter?.kind === "livekit") {
              void startSfu(channelId, mySessionId, currentGen, false);
            } else if (!adapter && roomModeRef.current === "sfu") {
              // アダプタを作る前（join の応答待ち・LiveKit の読み込み中）なら、覚えておいて作ったあとに移る。
              sfuReassignPendingRef.current = true;
            }
            return;
          }

          if (eventType === "participant_joined") {
            const newP = isCallParticipant(data.participant) ? data.participant : null;
            if (newP && newP.session_id !== mySessionId) {
              // 既に退場済みのセッションの参加通知は無視 ([P2])
              if (leftSessionIdsRef.current.has(newP.session_id)) return;

              if (
                shouldPlayRemoteJoin(
                  newP.session_id,
                  mySessionId,
                  knownSessionIdsRef.current,
                  leftSessionIdsRef.current,
                )
              ) {
                playCallSound("join");
              }
              knownSessionIdsRef.current.add(newP.session_id);

              sessionUserMapRef.current.set(newP.session_id, newP.user_id);
              setParticipants((prev) => {
                if (prev.some((p) => p.session_id === newP.session_id)) return prev;
                return [...prev, newP];
              });
              void pollActiveCalls();

              // SFU モードでは mesh の接続を作らない。
              if (roomModeRef.current === "sfu") return;
              // 既にピアが存在する場合は重複作成・送信を防止 ([P2])
              if (peersRef.current.has(newP.session_id)) return;

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
                  const offer = await createAudioOffer(pc);
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
              if (shouldPlayRemoteLeave(leftSid, mySessionId, knownSessionIdsRef.current)) {
                playCallSound("leave");
              }
              knownSessionIdsRef.current.delete(leftSid);
              leftSessionIdsRef.current.add(leftSid);
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

            if (!fromSid || !signal) return;
            // SFU モードでは mesh のシグナルを受けない。
            if (roomModeRef.current === "sfu") return;

            if (signal.type === "offer" && signal.sdp) {
              const lastHandled = lastHandledOffersRef.current.get(fromSid);
              const signalId = (signal as { signal_id?: string }).signal_id;
              const isDuplicate =
                lastHandled &&
                ((signalId && lastHandled.signalId === signalId) || lastHandled.sdp === signal.sdp);

              const existingEntry = peersRef.current.get(fromSid);
              if (isDuplicate && existingEntry && existingEntry.pc.signalingState !== "closed") {
                // 再送された同一 Offer の重複処理を防止 ([P2]):
                // 既存の PeerConnection を破壊せず、生成済み Answer を再送するか処理をスキップする
                if (lastHandled.answerSdp) {
                  try {
                    await sendSignalWithRetry(channelId, mySessionId, fromSid, {
                      type: "answer",
                      sdp: lastHandled.answerSdp,
                    });
                  } catch (reErr) {
                    console.error("Failed to resend answer for duplicated offer", reErr);
                  }
                }
                return;
              }

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
                const entry = peersRef.current.get(fromSid);
                const answer = await answerAudioOffer(
                  pc,
                  signal.sdp,
                  entry?.pendingCandidates ?? [],
                );

                lastHandledOffersRef.current.set(fromSid, {
                  signalId,
                  sdp: signal.sdp,
                  answerSdp: answer.sdp,
                });

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
                  await applyAudioAnswer(entry.pc, signal.sdp, entry.pendingCandidates);
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
        };

        for await (const { eventType, data } of readCallEvents(response)) {
          if (callGenerationRef.current !== currentGen) return;
          if (eventType === "room_state") {
            const rawParticipants = Array.isArray(data.participants) ? data.participants : [];
            const roomParticipants = rawParticipants.filter(isCallParticipant);
            const ice = (data.ice_servers as RTCIceServer[]) ?? [
              { urls: "stun:stun.l.google.com:19302" },
            ];
            iceServersRef.current = ice;
            const roomMode: RoomMode = data.mode === "sfu" ? "sfu" : "mesh";

            // 先行して受信した participant_left / participant_joined とマージ整合性を担保 ([P2])
            const leftSessionIds = leftSessionIdsRef.current;
            const validRoomParticipants = roomParticipants.filter(
              (p) => !leftSessionIds.has(p.session_id),
            );

            for (const p of validRoomParticipants) {
              sessionUserMapRef.current.set(p.session_id, p.user_id);
              knownSessionIdsRef.current.add(p.session_id);
            }

            setParticipants((prev) => {
              const map = new Map<string, CallParticipant>();
              for (const p of validRoomParticipants) {
                const isSelf = p.session_id === mySessionId;
                map.set(p.session_id, isSelf ? { ...p, is_muted: isMutedRef.current } : p);
              }
              for (const p of prev) {
                if (!map.has(p.session_id) && !leftSessionIds.has(p.session_id)) {
                  const isSelf = p.session_id === mySessionId;
                  map.set(p.session_id, isSelf ? { ...p, is_muted: isMutedRef.current } : p);
                }
              }
              return Array.from(map.values());
            });

            // 接続完了前に設定されたミュート状態をバックエンドに即時反映 ([P2])
            // toggleMute と同一の直列化キュー（muteSerialQueueRef）を通して送信し、
            // 接続直後のミュート解除との並行送信による逆転・上書きを防止
            if (isMutedRef.current) {
              void syncMuteStateToServer(channelId, mySessionId);
            }

            // SFU モードの部屋なら、mesh を張らずに SFU につなぐ。
            if (roomMode === "sfu" && roomModeRef.current !== "sfu") {
              roomModeRef.current = "sfu";
              void startSfu(channelId, mySessionId, currentGen, false);
            }

            // 同時参加時の Offer 衝突 (Glare) 回避 & 冪等性ガード ([P2]):
            // 既に PeerConnection が存在する場合は重複作成しない。
            // セッション ID の辞書順比較で小さい側のみが Offer を送信する。
            for (const p of roomModeRef.current === "sfu" ? [] : validRoomParticipants) {
              if (callGenerationRef.current !== currentGen) return;
              if (p.session_id === mySessionId) continue;
              if (peersRef.current.has(p.session_id)) continue;
              if (mySessionId.localeCompare(p.session_id) < 0) {
                const pc = createPeer(channelId, mySessionId, p.session_id, p.user_id, ice);
                try {
                  const offer = await pc.createOffer();
                  if (callGenerationRef.current !== currentGen) {
                    pc.close();
                    return;
                  }
                  await pc.setLocalDescription(offer);
                  if (callGenerationRef.current !== currentGen) {
                    pc.close();
                    return;
                  }
                  await sendSignalWithRetry(channelId, mySessionId, p.session_id, {
                    type: "offer",
                    sdp: offer.sdp,
                  });
                  if (callGenerationRef.current !== currentGen) {
                    pc.close();
                    return;
                  }
                } catch (offerErr) {
                  console.error("Failed to send offer", offerErr);
                }
              }
            }

            if (callGenerationRef.current !== currentGen) return;
            hasReceivedRoomState = true;

            // room_state 到着前に受信した先行イベントを順次再生 ([P2])
            while (pendingEarlyEvents.length > 0) {
              if (callGenerationRef.current !== currentGen) return;
              const pending = pendingEarlyEvents.shift()!;
              await handleNonRoomStateEvent(pending.eventType, pending.data);
              if (callGenerationRef.current !== currentGen) return;
            }
          } else if (!hasReceivedRoomState) {
            // room_state 受信前の場合: participant_left は即時記録し、イベントは保留 ([P2])
            if (eventType === "participant_left") {
              const leftSid = data.session_id as string;
              if (leftSid) {
                leftSessionIdsRef.current.add(leftSid);
              }
            }
            pendingEarlyEvents.push({ eventType, data });
          } else {
            await handleNonRoomStateEvent(eventType, data);
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
      enqueueMicSwitch,
      startSfu,
      syncMuteStateToServer,
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
        selectedMicId,
        selectedSpeakerId,
        setMicrophone,
        setSpeaker,
        getCallDiagnostics,
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
