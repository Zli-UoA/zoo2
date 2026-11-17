import { useEffect } from "react";
import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vite-plus/test";
import type { AppConfig } from "../../lib/config";
import { notifyAuthSessionEnded } from "../../lib/auth";
import { SipProvider, useSip } from "./SipProvider";
const mocks = vi.hoisted(() => ({
  get: vi.fn(),
  post: vi.fn(),
  disconnect: vi.fn(async () => {}),
  user: { currentUserId: "alice" as string | null },
  call: { currentChannelId: null as string | null },
}));
vi.mock("../../lib/api/client", () => ({
  createConfiguredApiClient: () => ({ get: mocks.get, post: mocks.post }),
}));
vi.mock("../../hooks/useCurrentUser", () => ({ useCurrentUser: () => mocks.user }));
vi.mock("../../lib/call/call-context", () => ({ useCall: () => mocks.call }));
vi.mock("../../lib/sip/driver", () => ({
  createPhoneDriverFactory: () => async (_c: unknown, events: { ready(): void }) => ({
    connect: async () => events.ready(),
    disconnect: mocks.disconnect,
  }),
}));
const config = {} as AppConfig;
function Consumer({ name }: { name: string }) {
  const sip = useSip()!;
  return (
    <div>
      <button onClick={() => void sip.refresh()}>refresh-{name}</button>
      <button onClick={() => void sip.controller.connect("home")}>connect-{name}</button>
      <button
        onClick={() =>
          void sip.save(null, {
            name: "Test",
            shared: false,
            websocket_path: "/sip/ws",
            sip_domain: "pbx.example",
            username: "alice",
            password: "dummy",
            phone_number: "",
            member_ids: [],
            phones: [],
          })
        }
      >
        save-{name}
      </button>
      <output aria-label={`state-${name}`}>
        {sip.state.status}:{sip.directory.lines.length}
      </output>
    </div>
  );
}
function App() {
  return (
    <SipProvider config={config}>
      <Consumer name="panel" />
      <Consumer name="banner" />
    </SipProvider>
  );
}
beforeEach(() => {
  vi.clearAllMocks();
  mocks.user = { currentUserId: "alice" };
  mocks.call = { currentChannelId: null };
  mocks.get.mockResolvedValue({ enabled: true, lines: [{ id: "home", can_connect: true }] });
  mocks.post.mockResolvedValue({
    data: {
      session_id: "00000000-0000-4000-8000-000000000001",
      peer_id: "00000000-0000-4000-8000-000000000002",
      revision: "v1",
      ice_servers: [],
    },
    txid: null,
  });
});
it("shares the directory request and SIP connection between panel and banner", async () => {
  let finish!: (value: unknown) => void;
  mocks.get.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  render(<App />);
  await userEvent.click(screen.getByRole("button", { name: "refresh-panel" }));
  await userEvent.click(screen.getByRole("button", { name: "refresh-banner" }));
  expect(mocks.get).toHaveBeenCalledTimes(1);
  await act(async () => finish({ enabled: true, lines: [{ id: "home", can_connect: true }] }));
  await userEvent.click(screen.getByRole("button", { name: "connect-panel" }));
  await expect.element(screen.getByLabelText("state-banner")).toHaveTextContent("ready:1");
  expect(mocks.post).toHaveBeenCalledTimes(1);
});
it("ignores a slow directory response after logout and disconnects the PBX", async () => {
  let finish!: (value: unknown) => void;
  render(<App />);
  await userEvent.click(screen.getByRole("button", { name: "connect-panel" }));
  mocks.get.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  await userEvent.click(screen.getByRole("button", { name: "refresh-panel" }));
  await act(async () => {
    notifyAuthSessionEnded("logout");
    finish({ enabled: true, lines: [{ id: "home" }] });
  });
  await expect.element(screen.getByLabelText("state-banner")).toHaveTextContent("offline:0");
  expect(mocks.disconnect).toHaveBeenCalledOnce();
});
it("clears both views when the account changes and prevents simultaneous channel audio", async () => {
  const view = render(<App />);
  await userEvent.click(screen.getByRole("button", { name: "refresh-panel" }));
  await userEvent.click(screen.getByRole("button", { name: "connect-panel" }));
  mocks.user = { currentUserId: "bob" };
  view.rerender(<App />);
  await expect.element(screen.getByLabelText("state-panel")).toHaveTextContent("offline:0");
  await userEvent.click(screen.getByRole("button", { name: "connect-panel" }));
  mocks.call = { currentChannelId: "channel" };
  view.rerender(<App />);
  await expect.element(screen.getByLabelText("state-panel")).toHaveTextContent("offline:0");
  expect(mocks.disconnect).toHaveBeenCalledTimes(2);
});

it("refreshes committed settings after an older directory request settles", async () => {
  let finish!: (value: unknown) => void;
  mocks.get.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  render(<App />);
  await userEvent.click(screen.getByRole("button", { name: "refresh-panel" }));
  await userEvent.click(screen.getByRole("button", { name: "save-panel" }));
  mocks.get.mockResolvedValue({ enabled: true, lines: [{ id: "home" }, { id: "new" }] });
  await act(async () => finish({ enabled: true, lines: [{ id: "home" }] }));
  await expect.element(screen.getByLabelText("state-panel")).toHaveTextContent("offline:2");
  expect(mocks.get).toHaveBeenCalledTimes(2);
});

function AutoConsumer() {
  const sip = useSip()!;
  useEffect(() => {
    if (!sip.loaded) void sip.refresh();
  }, [sip.refresh, sip.loaded, sip.resetCount]);
  return (
    <output aria-label="auto-directory">
      {sip.loaded ? sip.directory.lines[0]?.id : "loading"}
    </output>
  );
}
it("reloads after a reset while the initial directory is still pending", async () => {
  const pending: ((value: unknown) => void)[] = [];
  mocks.get.mockImplementation(
    () =>
      new Promise((resolve) => {
        pending.push(resolve);
      }),
  );
  const view = render(
    <SipProvider config={config}>
      <AutoConsumer />
    </SipProvider>,
  );
  await vi.waitFor(() => expect(pending.length).toBeGreaterThan(0));
  const before = mocks.get.mock.calls.length;
  mocks.user = { currentUserId: "bob" };
  view.rerender(
    <SipProvider config={config}>
      <AutoConsumer />
    </SipProvider>,
  );
  await vi.waitFor(() => expect(mocks.get.mock.calls.length).toBeGreaterThan(before));
  await act(async () => {
    for (const finish of pending.slice(0, -1)) finish({ enabled: true, lines: [{ id: "old" }] });
  });
  await expect.element(screen.getByLabelText("auto-directory")).toHaveTextContent("loading");
  await act(async () => pending.at(-1)!({ enabled: true, lines: [{ id: "bob-line" }] }));
  await expect.element(screen.getByLabelText("auto-directory")).toHaveTextContent("bob-line");
});
