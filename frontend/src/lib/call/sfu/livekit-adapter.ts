/**
 * LiveKit（zoo2 の k8s、LAN の中だけ）につなぐアダプタ（docs/calls-sfu.md）。
 *
 * 参加者の identity は call の session_id。sfu-bridge（identity が `bridge:` で始まる）が Cloudflare の人の声を
 * publish するときは、トラック名がその人の session_id になる。
 */
import {
  type LocalAudioTrack,
  type RemoteParticipant,
  type RemoteTrack,
  type RemoteTrackPublication,
  Room,
  RoomEvent,
  Track,
} from "livekit-client";

import type { SfuAdapter, SfuAdapterCallbacks } from "./types";

/** この時間で ICE がつながらなければ、Cloudflare でやり直す（LAN の外にいるなど）。 */
export const LIVEKIT_CONNECT_TIMEOUT_MS = 8000;

export function speakerOf(participantIdentity: string, trackName: string): string {
  return participantIdentity.startsWith("bridge:") ? trackName : participantIdentity;
}

export class LiveKitAdapter implements SfuAdapter {
  readonly kind = "livekit" as const;
  private room: Room;
  private localTrack: LocalAudioTrack | null = null;
  private speakers = new Map<string, string>();
  private closed = false;

  constructor(
    private readonly url: string,
    private readonly token: string,
    private readonly mic: MediaStreamTrack,
    private readonly callbacks: SfuAdapterCallbacks,
  ) {
    this.room = new Room({
      adaptiveStream: false,
      dynacast: false,
      disconnectOnPageLeave: false,
      // マイクは通話側が持っている（Cloudflare でやり直すときも同じトラックを使う）ので止めない。
      stopLocalTrackOnUnpublish: false,
    });
    this.room.on(
      RoomEvent.TrackSubscribed,
      (track: RemoteTrack, publication: RemoteTrackPublication, participant: RemoteParticipant) => {
        if (track.kind !== Track.Kind.Audio) return;
        const speaker = speakerOf(participant.identity, publication.trackName);
        this.speakers.set(publication.trackSid, speaker);
        this.callbacks.onRemoteAudio(speaker, new MediaStream([track.mediaStreamTrack]));
      },
    );
    this.room.on(RoomEvent.TrackUnsubscribed, (_track, publication) => {
      const speaker = this.speakers.get(publication.trackSid);
      if (!speaker) return;
      this.speakers.delete(publication.trackSid);
      this.callbacks.onRemoteGone(speaker);
    });
    this.room.on(RoomEvent.Disconnected, () => {
      if (!this.closed) this.callbacks.onFailed("livekit_disconnected");
    });
  }

  async start(): Promise<void> {
    await this.room.connect(this.url, this.token, {
      autoSubscribe: true,
      peerConnectionTimeout: LIVEKIT_CONNECT_TIMEOUT_MS,
      maxRetries: 0,
    });
    if (this.closed) return;
    const publication = await this.room.localParticipant.publishTrack(this.mic, {
      source: Track.Source.Microphone,
      dtx: true,
      red: true,
    });
    this.localTrack = (publication.track as LocalAudioTrack | undefined) ?? null;
  }

  updateTracks(): void {
    // LiveKit はルームの購読だけで足りる。
  }

  async replaceMicTrack(track: MediaStreamTrack): Promise<void> {
    if (!this.localTrack) throw new Error("Microphone is not published yet");
    await this.localTrack.replaceTrack(track, { userProvidedTrack: true });
  }

  async setMuted(muted: boolean): Promise<void> {
    if (!this.localTrack) return;
    if (muted) await this.localTrack.mute();
    else await this.localTrack.unmute();
  }

  peerConnections(): RTCPeerConnection[] {
    return [];
  }

  close(): void {
    this.closed = true;
    for (const speaker of new Set(this.speakers.values())) this.callbacks.onRemoteGone(speaker);
    this.speakers.clear();
    void this.room.disconnect();
  }
}
