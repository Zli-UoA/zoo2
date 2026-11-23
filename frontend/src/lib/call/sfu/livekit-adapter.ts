/**
 * LiveKit（zoo2 の k8s、LAN の中だけ）につなぐアダプタ（docs/calls-sfu.md）。
 *
 * 参加者の identity は割り当てごとの `{session_id}.{n}`（入り直すたびに n が変わる。backend の
 * `livekit::participant_identity`）で、最初の `.` より前が call の session_id。sfu-bridge（identity が `bridge:` で
 * 始まる）が Cloudflare の人の声を publish するときは、トラック名がその人の session_id になる。
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

/** LiveKit の参加者とトラックから、話者の session_id を求める。 */
export function speakerOf(participantIdentity: string, trackName: string): string {
  if (participantIdentity.startsWith("bridge:")) return trackName;
  const dot = participantIdentity.indexOf(".");
  return dot < 0 ? participantIdentity : participantIdentity.slice(0, dot);
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
    private mic: MediaStreamTrack,
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
    const published = this.mic;
    const publication = await this.room.localParticipant.publishTrack(published, {
      source: Track.Source.Microphone,
      dtx: true,
      red: true,
    });
    this.localTrack = (publication.track as LocalAudioTrack | undefined) ?? null;
    // publish 中にマイクが切り替わっていたら、今のマイクに差し替える。
    if (this.localTrack && this.mic !== published) {
      await this.localTrack.replaceTrack(this.mic, { userProvidedTrack: true });
    }
  }

  updateTracks(): void {
    // LiveKit はルームの購読だけで足りる。
  }

  async replaceMicTrack(track: MediaStreamTrack): Promise<void> {
    // まだ publish していなければ、start() が publish するトラックを差し替える。
    this.mic = track;
    if (!this.localTrack) return;
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
    // マイクは通話側のもの（Cloudflare に移るときも使う）なので、切断で止めない。
    void this.room.disconnect(false);
  }
}
