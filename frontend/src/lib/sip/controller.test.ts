import { describe, expect, it, vi } from "vite-plus/test";
import type { ApiClient } from "../api/client";
import { SipController, dialTarget, type SipDriver, type SipEvents } from "./controller";
import type { SipCredentials } from "./types";
const credentials: SipCredentials = {
  websocket_path: "/sip/ws",
  sip_domain: "pbx.example",
  username: "alice",
  password: "dummy",
  revision: "v1",
  ice_servers: [],
};
function setup() {
  let events!: SipEvents;
  const driver: SipDriver = {
    connect: vi.fn(async () => {
      events.ready();
    }),
    disconnect: vi.fn(async () => {}),
    call: vi.fn(async () => {}),
    answer: vi.fn(async () => {
      events.answered();
    }),
    hangup: vi.fn(async () => {
      events.ended();
    }),
    mute: vi.fn(),
    dtmf: vi.fn(async () => {}),
  };
  const post = vi.fn(async () => ({ data: credentials, txid: null }));
  const factory = vi.fn(async (_: SipCredentials, e: SipEvents) => {
    events = e;
    return driver;
  });
  const controller = new SipController({ post } as unknown as ApiClient, factory);
  return { controller, driver, post, factory, events: () => events };
}
describe("SIP controller", () => {
  it("restricts dial strings to numbers on the configured PBX", () => {
    expect(dialTarget("+81312345678", "pbx.example")).toBe("sip:+81312345678@pbx.example");
    for (const n of [
      "",
      "+",
      "123@evil.example",
      "123;transport=udp",
      "1\r\nVia:x",
      "*98",
      "1".repeat(33),
    ])
      expect(() => dialTarget(n, "pbx.example")).toThrow();
    expect(() => dialTarget("123", "pbx.example;foo")).toThrow();
  });
  it("shares one transport and supports dial, mute, DTMF and hangup", async () => {
    const s = setup();
    await s.controller.connect("personal");
    expect(s.controller.snapshot().status).toBe("ready");
    await s.controller.dial("101");
    expect(s.post).toHaveBeenCalledTimes(2);
    expect(s.driver.call).toHaveBeenCalledWith("sip:101@pbx.example");
    s.events().answered();
    s.controller.toggleMute();
    expect(s.driver.mute).toHaveBeenCalledWith(true);
    await s.controller.dtmf("#");
    await s.controller.dtmf("invalid");
    expect(s.driver.dtmf).toHaveBeenCalledTimes(1);
    await s.controller.hangup();
    expect(s.controller.snapshot()).toMatchObject({ status: "ready", muted: false, peer: "" });
    expect(s.factory).toHaveBeenCalledTimes(1);
    s.controller.disconnect();
  });
  it("requires registration acknowledgement, rather than just an open WebSocket", async () => {
    vi.useFakeTimers();
    const s = setup();
    vi.mocked(s.driver.connect).mockResolvedValue();
    await s.controller.connect("personal");
    expect(s.controller.snapshot().status).toBe("connecting");
    await vi.advanceTimersByTimeAsync(15000);
    expect(s.controller.snapshot()).toMatchObject({ status: "offline", error: true });
    expect(s.driver.disconnect).toHaveBeenCalled();
    vi.useRealTimers();
  });
  it("drops a delayed credential response after logout or account switch", async () => {
    const s = setup();
    let finish!: (value: { data: SipCredentials; txid: null }) => void;
    s.post.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    );
    const connecting = s.controller.connect("private");
    s.controller.disconnect();
    finish({ data: credentials, txid: null });
    await connecting;
    expect(s.factory).not.toHaveBeenCalled();
    expect(s.controller.snapshot().status).toBe("offline");
  });
  it("disposes a late driver and ignores old-line callbacks after switching", async () => {
    const s = setup();
    let finish!: (value: SipDriver) => void;
    s.factory.mockImplementationOnce(async (_c, e) => {
      return new Promise((resolve) => {
        finish = resolve;
        e.disconnected();
      });
    });
    const connecting = s.controller.connect("private");
    await Promise.resolve();
    await Promise.resolve();
    finish(s.driver);
    await connecting;
    expect(s.driver.disconnect).toHaveBeenCalled();
    expect(s.driver.connect).not.toHaveBeenCalled();
  });
  it("does not place a call after membership recheck fails", async () => {
    const s = setup();
    await s.controller.connect("shared");
    s.post.mockRejectedValueOnce(new Error("403"));
    await s.controller.dial("101");
    expect(s.driver.call).not.toHaveBeenCalled();
    expect(s.controller.snapshot().error).toBe(true);
    s.controller.disconnect();
  });
  it("clears the old registration before switching lines", async () => {
    const s = setup();
    await s.controller.connect("personal");
    const old = s.events();
    await s.controller.connect("shared");
    old.incoming();
    old.disconnected();
    expect(s.driver.disconnect).toHaveBeenCalledTimes(1);
    expect(s.controller.snapshot()).toMatchObject({ lineId: "shared", status: "ready" });
    s.controller.disconnect();
  });
  it("handles incoming calls and serializes operations", async () => {
    const s = setup();
    await s.controller.connect("personal");
    s.events().incoming();
    expect(s.controller.snapshot().status).toBe("incoming");
    await Promise.all([s.controller.answer(), s.controller.answer()]);
    expect(s.driver.answer).toHaveBeenCalledTimes(1);
    expect(s.controller.snapshot().status).toBe("active");
    s.controller.disconnect();
  });
});
