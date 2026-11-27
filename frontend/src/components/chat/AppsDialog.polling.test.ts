// @vitest-environment happy-dom
import { act, createElement, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vite-plus/test";
import { AppsDialog } from "./AppsDialog";

const api = vi.hoisted(() => ({ config: {}, get: vi.fn(), request: vi.fn() }));
vi.mock("../../lib/api/client", () => ({ createConfiguredApiClient: () => api }));
vi.mock("../../lib/electric/collections-context", () => ({ useCollections: () => api }));
vi.mock("../../hooks/useCurrentUser", () => ({
  useCurrentUser: () => ({ currentUserId: "me", isAdmin: false }),
}));
vi.mock("../../hooks/useRealtime", () => ({
  useUsers: () => ({ data: [] }),
  useActiveChannels: () => ({ data: [] }),
}));
vi.mock("../layout/shell-context", () => ({ useShell: () => ({ selectedChannelId: "general" }) }));
vi.mock("@tanstack/react-router", () => ({ useNavigate: () => vi.fn() }));
vi.mock("react-i18next", () => {
  const t = (key: string) => key;
  return { useTranslation: () => ({ t }) };
});
vi.mock("../ui/dialog", () => {
  const content = ({ children }: { children: ReactNode }) => createElement("div", null, children);
  return Object.fromEntries(
    ["Dialog", "DialogContent", "DialogDescription", "DialogHeader", "DialogTitle"].map((name) => [
      name,
      content,
    ]),
  );
});
vi.mock("@base-ui/react/tabs", () => {
  const content = ({ children }: { children: ReactNode }) => createElement("div", null, children);
  return { Tabs: { Root: content, List: content, Tab: content, Panel: content } };
});
vi.mock("../ui/select", () => ({ Select: () => null }));
vi.mock("./AppBlocks", () => ({ AppBlocks: () => createElement("p", null, "Published home") }));
const container = document.createElement("div");
const root = createRoot(container);
afterEach(async () => {
  await act(async () => root.unmount());
  vi.useRealTimers();
  vi.unstubAllGlobals();
});
it("wakes backed-off home polling after invoke and watches for asynchronous publication", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "Date"] });
  let published = false;
  let surfaceCalls = 0;
  api.get.mockImplementation((path: string) => {
    if (path === "/api/apps")
      return Promise.resolve([
        {
          id: "alpha",
          name: "Alpha",
          description: "",
          owner_id: "other",
          status: "installed",
          manifest: {},
          revision: 1,
        },
      ]);
    if (path === "/api/apps/alpha/surfaces") {
      surfaceCalls++;
      return Promise.resolve(
        published ? [{ id: "home", kind: "home", document: { blocks: [] } }] : [],
      );
    }
    return Promise.resolve([]);
  });
  api.request.mockResolvedValue({});
  await act(async () =>
    root.render(createElement(AppsDialog, { initialId: "alpha", onClose: () => {} })),
  );
  await act(async () => vi.advanceTimersByTimeAsync(28_000));
  expect(surfaceCalls).toBe(4);
  const openHome = [...container.querySelectorAll("button")].find(
    (button) => button.textContent === "apps.openHome",
  );
  expect(openHome).toBeDefined();
  await act(async () => openHome!.click());
  expect(api.request).toHaveBeenCalledWith(
    "/api/apps/alpha/invoke",
    expect.objectContaining({ body: expect.objectContaining({ kind: "home" }) }),
  );
  expect(surfaceCalls).toBe(5);
  published = true;
  await act(async () => vi.advanceTimersByTimeAsync(2_000));
  expect(surfaceCalls).toBe(6);
  expect(container.textContent).toContain("Published home");
});
