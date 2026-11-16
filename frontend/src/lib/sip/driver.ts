import type { ApiClient } from "../api/client";
import type { AppConfig } from "../config";
import { AudioCallMedia } from "../call/audio-media";
import { acquireCallMicrophone } from "../call/rtc-audio";
import { openCallStream, readCallEvents } from "../call/signaling";
import { applySink, loadSelectedDevice } from "../call/call-devices";
import type { SipDriverFactory } from "./controller";

function deferred() {
  let resolve!: () => void;
  let reject!: (reason: Error) => void;
  const promise = new Promise<void>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  // Calls can be cancelled while a microphone permission prompt is pending.
  void promise.catch(() => {});
  return { promise, resolve, reject };
}

/** Plain authenticated HTTP/SSE and the same WebRTC audio core as ordinary calls. */
export function createPhoneDriverFactory(api: ApiClient, config: AppConfig): SipDriverFactory {
  return async (connection, events) => {
    if (!/^[a-f0-9-]{36}$/.test(connection.session_id) || !connection.peer_id)
      throw new Error("Invalid phone connection");
    const path = `/api/phone/sessions/${encodeURIComponent(connection.session_id)}`;
    const abort = new AbortController();
    let closed = false;
    let generation = 0;
    let submittedGeneration: number | null = null;
    let media: AudioCallMedia | null = null;
    let incoming: Record<string, unknown> | null = null;
    let accepted = false;
    let connected = false;
    let notified = false;
    let descriptionSent = false;
    let description: ReturnType<typeof deferred> | null = null;
    let callNumber = "";
    let candidates: unknown[] = [];
    let earlySignals: Record<string, unknown>[] = [];
    let queue: Promise<unknown> = Promise.resolve();
    let ending: ReturnType<typeof deferred> | null = null;
    const active = () => {
      if (accepted && connected && !notified && !closed) {
        notified = true;
        events.answered();
      }
    };
    const stopMedia = () => {
      ++generation;
      media?.close();
      media = null;
      candidates = [];
      earlySignals = [];
      callNumber = "";
      description?.reject(new Error("Phone call ended"));
      description = null;
      accepted = false;
      connected = false;
      notified = false;
      descriptionSent = false;
    };
    const disconnect = async () => {
      if (closed) return;
      closed = true;
      abort.abort();
      stopMedia();
      incoming = null;
      ending?.resolve();
      ending = null;
      const body = { action: "disconnect" };
      if (api.keepalive) await api.keepalive(`${path}/command`, body).catch(() => {});
      else await api.post(`${path}/command`, body, { keepalive: true }).catch(() => {});
    };
    const fail = () => {
      if (!closed) {
        void disconnect();
        events.disconnected();
      }
    };
    const send = (body: Record<string, unknown>, mine?: number) => {
      const request = queue.then(async () => {
        if (closed || (mine !== undefined && mine !== generation)) return;
        if (body.action === "call" || body.action === "accept") submittedGeneration = mine ?? null;
        await api.post(`${path}/command`, body, { signal: abort.signal });
      });
      queue = request.catch(() => {});
      return request;
    };
    const signal = (value: Record<string, unknown>, mine: number) => {
      const rtc = value.signal as Record<string, unknown>;
      if (mine !== generation || closed) return;
      const failIfCurrent = () => {
        if (mine === generation) fail();
      };
      if (rtc.type === "candidate") {
        if (!descriptionSent) {
          if (candidates.length >= 128) {
            fail();
            return;
          }
          candidates.push(rtc);
        } else void send({ action: "candidate", signal: rtc }, mine).catch(failIfCurrent);
      } else if (rtc.type === "offer" || rtc.type === "answer") {
        const done = description;
        void send(
          {
            action: rtc.type === "offer" ? "call" : "accept",
            ...(rtc.type === "offer" ? { number: callNumber } : {}),
            signal: rtc,
          },
          mine,
        )
          .then(() => {
            if (mine !== generation || closed) return;
            descriptionSent = true;
            for (const candidate of candidates.splice(0))
              void send({ action: "candidate", signal: candidate }, mine).catch(failIfCurrent);
            done?.resolve();
          })
          .catch(() => {
            done?.reject(new Error("Phone operation failed"));
            if (mine === generation) fail();
          });
      }
    };
    const start = async (number?: string) => {
      if (closed || media) throw new Error("Phone call unavailable");
      const mine = ++generation;
      const offer = incoming;
      callNumber = number ?? "";
      accepted = false;
      connected = false;
      notified = false;
      descriptionSent = false;
      candidates = [];
      description = deferred();
      const done = description;
      const next = new AudioCallMedia(
        connection.session_id,
        connection.ice_servers,
        (event, payload) => {
          if (mine !== generation || closed) return;
          if (event === "signal") signal(payload as Record<string, unknown>, mine);
          else if (event === "connected") {
            connected = true;
            active();
          } else if (event === "failed") fail();
        },
        async () => (await acquireCallMicrophone(loadSelectedDevice("mic"))).stream,
        connection.relay_only !== false,
        async (audio) => {
          await applySink(audio, loadSelectedDevice("speaker"));
        },
      );
      media = next;
      try {
        await next.start();
        if (mine !== generation || closed) return;
        await next.event("room_state", {
          participants: [{ session_id: connection.peer_id }],
          offerer_session_id: number ? connection.session_id : connection.peer_id,
        });
        if (offer)
          await next.event("signal", { from_session_id: connection.peer_id, signal: offer });
        for (const pending of earlySignals.splice(0)) await next.event("signal", pending);
        let timeout!: ReturnType<typeof setTimeout>;
        try {
          await Promise.race([
            done.promise,
            new Promise<never>((_, reject) => {
              timeout = setTimeout(() => reject(new Error("Phone signaling timed out")), 15000);
            }),
          ]);
        } finally {
          clearTimeout(timeout);
        }
      } catch (error) {
        if (mine === generation) {
          // A submitted offer may still create a SIP call after a client timeout.
          // Revoke the whole session so server cleanup also terminates that call.
          if (submittedGeneration === mine) fail();
          else stopMedia();
        }
        throw error;
      }
    };
    return {
      connect: async () => {
        const response = await openCallStream(config, `${path}/stream`, {
          signal: abort.signal,
          method: "POST",
        });
        if (!response.ok) throw new Error("Phone stream unavailable");
        void (async () => {
          for await (const { eventType, data } of readCallEvents(response)) {
            if (closed) return;
            if (eventType === "registered") events.ready();
            else if (eventType === "incoming") {
              if (!media && !incoming) {
                incoming = data.signal as Record<string, unknown>;
                events.incoming();
              }
            } else if (eventType === "signal") {
              if (media) await media.event("signal", data);
              else if (incoming) {
                if (earlySignals.length >= 128) throw new Error("Too many phone signals");
                earlySignals.push(data);
              }
            } else if (eventType === "accepted") {
              incoming = null;
              accepted = true;
              active();
            } else if (eventType === "ended") {
              stopMedia();
              incoming = null;
              ending?.resolve();
              ending = null;
              events.ended();
            } else if (eventType === "disconnected") {
              fail();
              return;
            }
          }
          if (!closed) fail();
        })().catch(() => {
          if (!closed) fail();
        });
      },
      disconnect,
      call: (number) => start(number),
      answer: () => {
        if (!incoming) return Promise.reject(new Error("Phone invitation ended"));
        return start();
      },
      hangup: async () => {
        if (closed || (!media && !incoming)) return;
        const decline = !media && incoming !== null;
        if (media && !descriptionSent) {
          // Wait for the queued description before sending its hangup.
          await description?.promise.catch(() => {});
          if (closed) return;
        }
        ending = deferred();
        const done = ending;
        media?.close();
        let timeout!: ReturnType<typeof setTimeout>;
        try {
          await send({ action: decline ? "decline" : "hangup" });
          await Promise.race([
            done.promise,
            new Promise<never>((_, reject) => {
              timeout = setTimeout(() => reject(new Error("Phone hangup timed out")), 5000);
            }),
          ]);
        } catch (error) {
          fail();
          throw error;
        } finally {
          clearTimeout(timeout);
        }
      },
      mute: (muted) => media?.mute(muted),
      dtmf: (tone) => send({ action: "dtmf", tone }).then(() => undefined),
    };
  };
}
