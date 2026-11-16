import { afterEach, expect, it, vi } from "vite-plus/test";
import type { ApiClient } from "../api/client";
import type { AppConfig } from "../config";
import { createPhoneDriverFactory } from "./driver";

const connection = {
  session_id: "00000000-0000-4000-8000-000000000001",
  peer_id: "00000000-0000-4000-8000-000000000002",
  ice_servers: [],
  revision: "v1",
};
const cleanup: (() => void | Promise<void>)[] = [];
afterEach(async () => {
  for (const close of cleanup.splice(0).reverse()) await close();
  vi.restoreAllMocks();
});
async function fixture(relayOnly = true) {
  const audio = new AudioContext();
  const source = audio.createMediaStreamDestination().stream;
  const original = window.RTCPeerConnection;
  const server = new original();
  server.addTrack(source.getAudioTracks()[0], source);
  const peers: RTCPeerConnection[] = [];
  vi.spyOn(window, "RTCPeerConnection").mockImplementation(function (config) {
    const peer = new original(config);
    peers.push(peer);
    return peer;
  });
  const mic = vi
    .spyOn(navigator.mediaDevices, "getUserMedia")
    .mockImplementation(async () => new MediaStream(source.getAudioTracks().map((t) => t.clone())));
  let output!: ReadableStreamDefaultController<Uint8Array>;
  const stream = new ReadableStream<Uint8Array>({
    start(controller) {
      output = controller;
    },
  });
  const send = (event: string, data: unknown = {}) =>
    output.enqueue(new TextEncoder().encode(`event: ${event}\ndata: ${JSON.stringify(data)}\n\n`));
  vi.spyOn(window, "fetch").mockResolvedValue(
    new Response(stream, { headers: { "Content-Type": "text/event-stream" } }),
  );
  const post = vi.fn(async (_path: string, body: unknown) => {
    const command = body as { action: string; signal?: RTCSessionDescriptionInit };
    if (command.action === "call") {
      await server.setRemoteDescription(command.signal!);
      const answer = await server.createAnswer();
      await server.setLocalDescription(answer);
      send("signal", { from_session_id: connection.peer_id, signal: answer });
      send("accepted");
    } else if (command.action === "accept") {
      await server.setRemoteDescription(command.signal!);
      send("accepted");
    } else if (command.action === "hangup" || command.action === "decline") send("ended");
    return { data: {}, txid: null };
  });
  const events = {
    ready: vi.fn(),
    incoming: vi.fn(),
    answered: vi.fn(),
    ended: vi.fn(),
    disconnected: vi.fn(),
  };
  const driver = await createPhoneDriverFactory(
    { post } as unknown as ApiClient,
    { apiBaseUrl: "https://zoo.test", devUserId: "alice" } as AppConfig,
  )({ ...connection, relay_only: relayOnly }, events);
  cleanup.push(async () => {
    await driver.disconnect();
    server.close();
    source.getTracks().forEach((t) => t.stop());
    await audio.close();
  });
  await driver.connect();
  send("registered");
  await vi.waitFor(() => expect(events.ready).toHaveBeenCalledOnce());
  const connected = () => {
    Object.defineProperty(peers[0], "connectionState", { value: "connected", configurable: true });
    peers[0].dispatchEvent(new Event("connectionstatechange"));
  };
  return { driver, events, mic, server, send, post, connected, audio, peers };
}
it("uses HTTP/SSE and shared audio-only WebRTC, without capturing while registered", async () => {
  const f = await fixture();
  expect(f.mic).not.toHaveBeenCalled();
  await f.driver.call("101");
  expect(f.peers[0].getConfiguration().iceTransportPolicy).toBe("relay");
  const command = f.post.mock.calls.find(
    ([, body]) => (body as { action: string }).action === "call",
  )![1];
  expect(command).toMatchObject({ action: "call", number: "101", signal: { type: "offer" } });
  expect(JSON.stringify(command)).not.toMatch(/password|authorizationPassword|sip:/);
  expect(f.events.answered).not.toHaveBeenCalled();
  f.connected();
  await vi.waitFor(() => expect(f.events.answered).toHaveBeenCalledOnce());
  f.driver.mute(true);
  await f.driver.hangup();
  expect(f.events.ended).toHaveBeenCalledOnce();
});
it("defers incoming capture until answer and negotiates the gateway offer", async () => {
  const f = await fixture();
  const offer = await f.server.createOffer();
  await f.server.setLocalDescription(offer);
  f.send("incoming", { signal: offer });
  await vi.waitFor(() => expect(f.events.incoming).toHaveBeenCalledOnce());
  expect(f.mic).not.toHaveBeenCalled();
  await f.driver.answer();
  expect(
    f.post.mock.calls.some(([, body]) => (body as { action: string }).action === "accept"),
  ).toBe(true);
  f.connected();
  await vi.waitFor(() => expect(f.events.answered).toHaveBeenCalledOnce());
});
it("stops capture returned after disconnect without placing an external call", async () => {
  const f = await fixture();
  const late = f.audio.createMediaStreamDestination().stream;
  let finish!: (stream: MediaStream) => void;
  f.mic.mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const dialing = f.driver.call("101");
  await f.driver.disconnect();
  finish(late);
  await dialing;
  expect(late.getTracks().every((t) => t.readyState === "ended")).toBe(true);
  expect(f.post.mock.calls.some(([, body]) => (body as { action: string }).action === "call")).toBe(
    false,
  );
});

it("supports direct WebRTC within the backend LAN without TURN", async () => {
  const f = await fixture(false);
  await f.driver.call("101");
  expect(f.peers[0].getConfiguration().iceTransportPolicy).toBe("all");
  expect(f.peers[0].getConfiguration().iceServers).toEqual([]);
});

it("revokes the gateway session when a submitted call outlives signaling timeout", async () => {
  const f = await fixture();
  let finish!: () => void;
  const pending = new Promise<void>((resolve) => {
    finish = resolve;
  });
  const original = f.post.getMockImplementation()!;
  f.post.mockImplementation(async (path, body) => {
    if ((body as { action: string }).action === "call") await pending;
    return original(path, body);
  });
  const call = f.driver.call("101");
  const failure = expect(call).rejects.toThrow("Phone signaling timed out");
  await vi.waitFor(() =>
    expect(
      f.post.mock.calls.some(([, body]) => (body as { action: string }).action === "call"),
    ).toBe(true),
  );
  await failure;
  expect(
    f.post.mock.calls.some(([, body]) => (body as { action: string }).action === "disconnect"),
  ).toBe(true);
  expect(f.events.disconnected).toHaveBeenCalledOnce();
  finish();
  await expect(f.driver.call("102")).rejects.toThrow("Phone call unavailable");
}, 20000);

it("ignores a candidate failure from the previous call", async () => {
  const f = await fixture();
  await f.driver.call("101");
  let reject!: (error: Error) => void;
  const pending = new Promise<never>((_, no) => {
    reject = no;
  });
  const original = f.post.getMockImplementation()!;
  f.post.mockImplementation((path, body) =>
    (body as { action: string }).action === "candidate" ? pending : original(path, body),
  );
  f.peers[0].dispatchEvent(
    new RTCPeerConnectionIceEvent("icecandidate", {
      candidate: new RTCIceCandidate({
        candidate: "candidate:1 1 UDP 1 192.0.2.1 1234 typ host",
        sdpMid: "0",
        sdpMLineIndex: 0,
      }),
    }),
  );
  await vi.waitFor(() =>
    expect(
      f.post.mock.calls.some(([, body]) => (body as { action: string }).action === "candidate"),
    ).toBe(true),
  );
  f.send("ended");
  await vi.waitFor(() => expect(f.events.ended).toHaveBeenCalledOnce());
  const next = f.driver.call("102");
  reject(new Error("old candidate failed"));
  await next;
  expect(f.events.disconnected).not.toHaveBeenCalled();
  expect(
    f.post.mock.calls.filter(([, body]) => (body as { action: string }).action === "call"),
  ).toHaveLength(2);
});

it("disconnects the session when hangup is rejected", async () => {
  const f = await fixture();
  await f.driver.call("101");
  f.connected();
  await vi.waitFor(() => expect(f.events.answered).toHaveBeenCalledOnce());
  const original = f.post.getMockImplementation()!;
  f.post.mockImplementation((path, body) =>
    (body as { action: string }).action === "hangup"
      ? Promise.reject(new Error("rejected hangup"))
      : original(path, body),
  );
  await expect(f.driver.hangup()).rejects.toThrow("rejected hangup");
  expect(f.events.disconnected).toHaveBeenCalledOnce();
  await expect(f.driver.call("102")).rejects.toThrow("Phone call unavailable");
});

it("ignores a rejected hangup after the old call already ended", async () => {
  const f = await fixture();
  await f.driver.call("101");
  f.connected();
  await vi.waitFor(() => expect(f.events.answered).toHaveBeenCalledOnce());
  let reject!: (error: Error) => void;
  const pending = new Promise<never>((_, no) => {
    reject = no;
  });
  const original = f.post.getMockImplementation()!;
  f.post.mockImplementation((path, body) =>
    (body as { action: string }).action === "hangup" ? pending : original(path, body),
  );
  const hangup = f.driver.hangup();
  const failure = expect(hangup).rejects.toThrow("old hangup failed");
  await vi.waitFor(() =>
    expect(
      f.post.mock.calls.some(([, body]) => (body as { action: string }).action === "hangup"),
    ).toBe(true),
  );
  f.send("ended");
  await vi.waitFor(() => expect(f.events.ended).toHaveBeenCalledOnce());
  const next = f.driver.call("102");
  reject(new Error("old hangup failed"));
  await failure;
  await next;
  expect(f.events.disconnected).not.toHaveBeenCalled();
});

it("drops queued DTMF when its call has ended", async () => {
  const f = await fixture();
  await f.driver.call("101");
  f.connected();
  await vi.waitFor(() => expect(f.events.answered).toHaveBeenCalledOnce());
  let release!: () => void;
  const pending = new Promise<void>((resolve) => {
    release = resolve;
  });
  const original = f.post.getMockImplementation()!;
  f.post.mockImplementation(async (path, body) => {
    if ((body as { action: string }).action === "candidate") await pending;
    return original(path, body);
  });
  f.peers[0].dispatchEvent(
    new RTCPeerConnectionIceEvent("icecandidate", {
      candidate: new RTCIceCandidate({
        candidate: "candidate:1 1 UDP 1 192.0.2.1 1234 typ host",
        sdpMid: "0",
        sdpMLineIndex: 0,
      }),
    }),
  );
  await vi.waitFor(() =>
    expect(
      f.post.mock.calls.some(([, body]) => (body as { action: string }).action === "candidate"),
    ).toBe(true),
  );
  const tone = f.driver.dtmf("1");
  f.send("ended");
  await vi.waitFor(() => expect(f.events.ended).toHaveBeenCalledOnce());
  const next = f.driver.call("102");
  await vi.waitFor(() => expect(f.peers).toHaveLength(2));
  release();
  await Promise.all([tone, next]);
  expect(f.post.mock.calls.some(([, body]) => (body as { action: string }).action === "dtmf")).toBe(
    false,
  );
  expect(f.events.disconnected).not.toHaveBeenCalled();
});
