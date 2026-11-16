/** Audio-only WebRTC mesh using zoo2's existing channel-call signaling protocol. */
interface Peer {
  pc: RTCPeerConnection;
  audio: HTMLAudioElement;
  candidates: RTCIceCandidateInit[];
  answer?: { offer: string; sdp: string };
}
export class DmMedia {
  private stream: MediaStream | null = null;
  private peers = new Map<string, Peer>();
  private early = new Map<string, RTCIceCandidateInit[]>();
  private left = new Set<string>();
  private members = new Set<string>();
  private queue: Promise<void> = Promise.resolve();
  private closed = false;
  private hadRemote = false;
  private connected = false;
  constructor(
    private session: string,
    private iceServers: RTCIceServer[],
    private emit: (event: string, payload?: unknown) => void,
    private acquire = () => navigator.mediaDevices.getUserMedia({ audio: true, video: false }),
  ) {}
  async start() {
    const stream = await this.acquire();
    if (this.closed) {
      stream.getTracks().forEach((track) => track.stop());
      return;
    }
    this.stream = stream;
  }
  event(event: string, data: Record<string, unknown>) {
    this.queue = this.queue
      .then(() => this.handle(event, data))
      .catch(() => {
        if (!this.closed) this.emit("failed");
      });
    return this.queue;
  }
  private peer(id: string): Peer {
    const existing = this.peers.get(id);
    if (existing) return existing;
    if (!this.stream || this.closed || this.peers.size >= 5 || this.left.has(id))
      throw new Error("Unavailable peer");
    const pc = new RTCPeerConnection({ iceServers: this.iceServers });
    const audio = document.createElement("audio");
    audio.autoplay = true;
    const peer = { pc, audio, candidates: this.early.get(id) ?? [] };
    this.early.delete(id);
    this.peers.set(id, peer);
    this.hadRemote = true;
    this.stream.getTracks().forEach((track) => pc.addTrack(track, this.stream!));
    pc.onicecandidate = (event) => {
      if (event.candidate && !this.closed)
        this.signal(id, { type: "candidate", candidate: event.candidate.toJSON() });
    };
    pc.ontrack = (event) => {
      audio.srcObject = event.streams[0] ?? new MediaStream([event.track]);
      void audio.play().catch(() => {});
    };
    pc.onconnectionstatechange = () => {
      if (this.closed) return;
      if (pc.connectionState === "connected" && !this.connected) {
        this.connected = true;
        this.emit("connected");
      }
      if (pc.connectionState === "failed") this.emit("failed");
    };
    return peer;
  }
  private signal(id: string, signal: unknown) {
    if (!this.closed) this.emit("signal", { target_session_id: id, signal });
  }
  private async join(id: string) {
    if (id === this.session || this.left.has(id) || this.peers.has(id)) return;
    this.members.add(id);
    this.hadRemote = true;
    if (this.session < id) {
      const p = this.peer(id);
      const offer = await p.pc.createOffer();
      await p.pc.setLocalDescription(offer);
      this.signal(id, { type: "offer", sdp: offer.sdp });
    }
  }
  private async flush(p: Peer) {
    for (const c of p.candidates.splice(0)) await p.pc.addIceCandidate(c);
  }
  private async handle(event: string, data: Record<string, unknown>) {
    if (this.closed) return;
    if (event === "room_state") {
      for (const p of data.participants as { session_id: string }[]) await this.join(p.session_id);
    } else if (event === "participant_joined") {
      await this.join((data.participant as { session_id: string }).session_id);
    } else if (event === "participant_left") {
      const id = data.session_id as string;
      this.left.add(id);
      this.members.delete(id);
      const p = this.peers.get(id);
      if (p) {
        p.pc.close();
        p.audio.pause();
        p.audio.srcObject = null;
        this.peers.delete(id);
      }
      this.early.delete(id);
      if (this.hadRemote && !this.members.size) this.emit("ended");
    } else if (event === "signal") {
      const id = data.from_session_id as string;
      if (!id || id === this.session || this.left.has(id)) return;
      const s = data.signal as { type: string; sdp?: string; candidate?: RTCIceCandidateInit };
      if (s.type === "offer" && s.sdp) {
        const p = this.peer(id);
        if (p.answer?.offer === s.sdp) {
          this.signal(id, { type: "answer", sdp: p.answer.sdp });
          return;
        }
        await p.pc.setRemoteDescription({ type: "offer", sdp: s.sdp });
        await this.flush(p);
        const answer = await p.pc.createAnswer();
        await p.pc.setLocalDescription(answer);
        p.answer = { offer: s.sdp, sdp: answer.sdp! };
        this.signal(id, { type: "answer", sdp: answer.sdp });
      } else if (s.type === "answer" && s.sdp) {
        const p = this.peers.get(id);
        if (p?.pc.signalingState !== "have-local-offer") return;
        await p.pc.setRemoteDescription({ type: "answer", sdp: s.sdp });
        await this.flush(p);
      } else if (s.type === "candidate" && s.candidate) {
        const p = this.peers.get(id);
        if (p?.pc.remoteDescription) await p.pc.addIceCandidate(s.candidate);
        else {
          const pending = p?.candidates ?? this.early.get(id) ?? [];
          if (pending.length >= 128 || this.early.size >= 6) return;
          pending.push(s.candidate);
          if (!p) this.early.set(id, pending);
        }
      }
    }
  }
  mute(muted: boolean) {
    this.stream?.getAudioTracks().forEach((track) => {
      track.enabled = !muted;
    });
  }
  close() {
    this.closed = true;
    this.stream?.getTracks().forEach((track) => track.stop());
    this.stream = null;
    this.peers.forEach((p) => {
      p.pc.close();
      p.audio.pause();
      p.audio.srcObject = null;
    });
    this.peers.clear();
    this.early.clear();
    this.members.clear();
  }
}
