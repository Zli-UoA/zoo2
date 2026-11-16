import { expect, it, vi } from "vite-plus/test";
import type { Web } from "sip.js";
import { createSipDriver } from "./driver";
const mocks = vi.hoisted(() => ({
  media: vi.fn(),
  factory: undefined as Web.MediaStreamFactory | undefined,
  stop: vi.fn(async () => {}),
}));
vi.mock("sip.js", () => ({
  Web: {
    defaultMediaStreamFactory: () => mocks.media,
    defaultSessionDescriptionHandlerFactory: (factory: Web.MediaStreamFactory) => {
      mocks.factory = factory;
      return vi.fn();
    },
    SimpleUser: class {
      delegate: unknown;
      localMediaStream = undefined;
      disconnect = mocks.stop;
    },
  },
}));
it("stops a microphone stream that arrives after logout/disconnect", async () => {
  const audio = new AudioContext();
  const stream = audio.createMediaStreamDestination().stream;
  const [track] = stream.getAudioTracks();
  let finish!: (stream: MediaStream) => void;
  mocks.media.mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const driver = await createSipDriver(
    {
      websocket_path: "/sip/ws",
      sip_domain: "pbx.example",
      username: "alice",
      password: "dummy",
      ice_servers: [],
      revision: "v1",
    },
    { ready: vi.fn(), incoming: vi.fn(), answered: vi.fn(), ended: vi.fn(), disconnected: vi.fn() },
  );
  const acquiring = mocks.factory!(
    { audio: true, video: false },
    {} as Web.SessionDescriptionHandler,
  );
  await driver.disconnect();
  finish(stream);
  await expect(acquiring).rejects.toThrow("SIP disconnected");
  expect(track.readyState).toBe("ended");
  expect(mocks.stop).toHaveBeenCalledOnce();
  await audio.close();
});
it("rejects a cross-origin transport before creating an account", async () => {
  await expect(
    createSipDriver(
      {
        websocket_path: "https://other.example/sip/ws",
        sip_domain: "pbx.example",
        username: "alice",
        password: "dummy",
        ice_servers: [],
        revision: "v1",
      },
      {
        ready: vi.fn(),
        incoming: vi.fn(),
        answered: vi.fn(),
        ended: vi.fn(),
        disconnected: vi.fn(),
      },
    ),
  ).rejects.toThrow("Invalid SIP WebSocket path");
});
