import { applySink, loadSelectedDevice } from "../call/call-devices";
import type { SipDriverFactory } from "./controller";

/** Loaded on explicit connection only. SIP messages and configuration must never enter logs. */
export const createSipDriver: SipDriverFactory = async (credentials, events) => {
  const { Web } = await import("sip.js");
  const audio = document.createElement("audio");
  audio.autoplay = true;
  await applySink(audio, loadSelectedDevice("speaker"));
  let stopped = false;
  const url = new URL(credentials.websocket_path, window.location.origin);
  if (url.origin !== window.location.origin || !url.pathname.startsWith("/sip/"))
    throw new Error("Invalid SIP WebSocket path");
  url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
  const user = new Web.SimpleUser(url.href, {
    aor: `sip:${credentials.username}@${credentials.sip_domain}`,
    reconnectionAttempts: 0,
    registererOptions: { expires: 120, logConfiguration: false },
    media: { constraints: { audio: true, video: false }, remote: { audio } },
    userAgentOptions: {
      sessionDescriptionHandlerFactory: Web.defaultSessionDescriptionHandlerFactory(
        async (constraints, handler) => {
          const mic = loadSelectedDevice("mic");
          const stream = await Web.defaultMediaStreamFactory()(
            {
              ...constraints,
              audio: constraints.audio && mic ? { deviceId: { ideal: mic } } : constraints.audio,
            },
            handler,
          );
          if (stopped) {
            stream.getTracks().forEach((track) => track.stop());
            throw new Error("SIP disconnected");
          }
          return stream;
        },
      ),
      authorizationUsername: credentials.username,
      authorizationPassword: credentials.password,
      logBuiltinEnabled: false,
      logConfiguration: false,
      transportOptions: { traceSip: false },
      sessionDescriptionHandlerFactoryOptions: {
        peerConnectionConfiguration: {
          iceServers: credentials.ice_servers,
          iceTransportPolicy: "relay",
        },
        iceGatheringTimeout: 8000,
      },
    },
    delegate: {
      onRegistered: events.ready,
      onUnregistered: events.disconnected,
      onServerDisconnect: events.disconnected,
      onCallReceived: events.incoming,
      onCallAnswered: () => {
        void audio.play().catch(() => {});
        events.answered();
      },
      onCallHangup: events.ended,
    },
    sendDTMFUsingSessionDescriptionHandler: true,
  });
  return {
    connect: async () => {
      await user.connect();
      await user.register();
    },
    disconnect: async () => {
      // Disable callbacks before teardown; disconnect also disposes sessions/media.
      stopped = true;
      user.delegate = undefined;
      user.localMediaStream?.getTracks().forEach((track) => track.stop());
      audio.pause();
      audio.srcObject = null;
      await user.disconnect();
    },
    call: async (target) => {
      await user.call(target);
    },
    answer: () => user.answer(),
    hangup: () => user.hangup(),
    mute: (muted) => {
      if (muted) user.mute();
      else user.unmute();
    },
    dtmf: (tone) => user.sendDTMF(tone),
  };
};
