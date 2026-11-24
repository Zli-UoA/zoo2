/**
 * LiveKit（zoo2 の k8s、LAN の中だけ）につなぐアダプタ（docs/calls-sfu.md）。
 *
 * 参加者の identity は割り当てごとの `{session_id}.{n}`（入り直すたびに n が変わる。backend の
 * `livekit::participant_identity`）で、最初の `.` より前が call の session_id。sfu-bridge（identity が `bridge:` で
 * 始まる）が Cloudflare の人の声を publish するときは、トラック名がその人の session_id になる。
 */
import {
  LocalAudioTrack,
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

/**
 * 話者ごとの購読中のトラック。同じ話者（session_id）に、前の割り当ての identity と新しい identity のトラックが
 * 同時にあることがある（前の identity を外す RemoveParticipant が届くまで）。話者に流す音声は、最後に購読した
 * トラック。前のトラックの購読が外れても、今流しているトラックでなければ何もしない。今流しているトラックが
 * 外れたら、残っている最新のトラックに戻し、残りがなければ話者がいなくなったことにする。
 */
export class SpeakerTracks {
  /** trackSid → 話者と音声。挿入順が購読順。 */
  private tracks = new Map<string, { speaker: string; stream: MediaStream }>();
  /** 話者 → 今流している trackSid。 */
  private active = new Map<string, string>();

  constructor(
    private readonly callbacks: Pick<SfuAdapterCallbacks, "onRemoteAudio" | "onRemoteGone">,
  ) {}

  subscribed(trackSid: string, speaker: string, stream: MediaStream): void {
    this.tracks.delete(trackSid);
    this.tracks.set(trackSid, { speaker, stream });
    this.active.set(speaker, trackSid);
    this.callbacks.onRemoteAudio(speaker, stream);
  }

  unsubscribed(trackSid: string): void {
    const entry = this.tracks.get(trackSid);
    if (!entry) return;
    this.tracks.delete(trackSid);
    if (this.active.get(entry.speaker) !== trackSid) return;
    let latest: [string, MediaStream] | null = null;
    for (const [sid, t] of this.tracks) {
      if (t.speaker === entry.speaker) latest = [sid, t.stream];
    }
    if (latest) {
      this.active.set(entry.speaker, latest[0]);
      this.callbacks.onRemoteAudio(entry.speaker, latest[1]);
    } else {
      this.active.delete(entry.speaker);
      this.callbacks.onRemoteGone(entry.speaker);
    }
  }

  clear(): void {
    for (const speaker of this.active.keys()) this.callbacks.onRemoteGone(speaker);
    this.active.clear();
    this.tracks.clear();
  }
}

export class LiveKitAdapter implements SfuAdapter {
  readonly kind = "livekit" as const;
  private room: Room;
  private localTrack: LocalAudioTrack | null = null;
  /**
   * LiveKit に渡すマイクの複製。SDK はトラックを包むときに `enabled` を true にし、ミュートで切り替える。mesh と
   * 共有するマイクそのものを渡すと、ミュート中でも mesh に音が漏れるので、複製を渡してこのアダプタが止める。
   */
  private ownMic: MediaStreamTrack | null = null;
  /** 最後に指定されたミュートの状態。publish の前に指定された分も、publish の前に当てる。 */
  private muted: boolean;
  private speakers: SpeakerTracks;
  private closed = false;

  constructor(
    private readonly url: string,
    private readonly token: string,
    private mic: MediaStreamTrack,
    private readonly callbacks: SfuAdapterCallbacks,
    /** `muted`: 作った時点のミュートの状態（ミュート中の通話から移るとき、publish の前にミュートする）。 */
    options: { muted?: boolean } = {},
  ) {
    this.muted = options.muted ?? false;
    this.speakers = new SpeakerTracks(callbacks);
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
        this.speakers.subscribed(
          publication.trackSid,
          speaker,
          new MediaStream([track.mediaStreamTrack]),
        );
      },
    );
    this.room.on(RoomEvent.TrackUnsubscribed, (_track, publication) => {
      this.speakers.unsubscribed(publication.trackSid);
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
    // マイクは通話側のもの（userProvidedTrack）の複製。ミュート中なら publish の前にミュートしておき、
    // publish した直後に音を送ったり、ミュートしていない状態を伝えたりしないようにする。
    const own = published.clone();
    this.ownMic = own;
    const local = new LocalAudioTrack(own, undefined, true);
    if (this.muted) await local.mute();
    if (this.closed) return;
    const publication = await this.room.localParticipant.publishTrack(local, {
      source: Track.Source.Microphone,
      dtx: true,
      red: true,
    });
    this.localTrack = (publication.track as LocalAudioTrack | undefined) ?? local;
    // publish 中にミュートが切り替わっていたら、今の状態に合わせる。
    if (this.localTrack.isMuted !== this.muted) await this.setMuted(this.muted);
    // publish 中にマイクが切り替わっていたら、今のマイクに差し替える。
    if (this.mic !== published) await this.replaceOwnMic(this.mic);
  }

  updateTracks(): void {
    // LiveKit はルームの購読だけで足りる。
  }

  async replaceMicTrack(track: MediaStreamTrack): Promise<void> {
    // まだ publish していなければ、start() が publish するトラックを差し替える。
    this.mic = track;
    if (!this.localTrack) return;
    await this.replaceOwnMic(track);
  }

  /** publish 中のトラックを、新しいマイクの複製に差し替え、前の複製を止める。 */
  private async replaceOwnMic(track: MediaStreamTrack): Promise<void> {
    if (!this.localTrack) return;
    const own = track.clone();
    const previous = this.ownMic;
    this.ownMic = own;
    await this.localTrack.replaceTrack(own, { userProvidedTrack: true });
    previous?.stop();
    if (this.closed) own.stop();
  }

  async setMuted(muted: boolean): Promise<void> {
    this.muted = muted;
    if (!this.localTrack) return;
    if (muted) await this.localTrack.mute();
    else await this.localTrack.unmute();
  }

  peerConnections(): RTCPeerConnection[] {
    return [];
  }

  close(): void {
    this.closed = true;
    this.speakers.clear();
    // マイクは通話側のもの（Cloudflare に移るときも使う）なので、切断で止めない。止めるのは複製だけ。
    void this.room.disconnect(false);
    this.ownMic?.stop();
  }
}
