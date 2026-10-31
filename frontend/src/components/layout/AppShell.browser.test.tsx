import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  createRootRoute,
  createRoute,
  createRouter,
  createMemoryHistory,
  RouterProvider,
  Outlet,
  Link,
} from "@tanstack/react-router";
import { describe, expect, it, vi } from "vite-plus/test";
import { page } from "vite-plus/test/browser/context";
import { AppShell } from "./AppShell";
import { useShell } from "./shell-context";

vi.mock("../preferences/SearchPreferences", () => ({
  SearchPreferencesProvider: ({ children }: { children: import("react").ReactNode }) => children,
}));
vi.mock("../pwa/PwaControls", () => ({
  PwaProvider: ({ children }: { children: import("react").ReactNode }) => children,
}));
vi.mock("../../hooks/usePresence", () => ({
  usePresence: () => {},
  useConversationView: () => {},
}));
vi.mock("../../hooks/useRealtime", () => ({
  useActiveChannels: () => ({ data: [], isLoading: false, isError: false }),
}));
vi.mock("../chat/AppSurfaceHost", () => ({ AppSurfaceHost: () => null }));
vi.mock("../chat/ChannelSidebar", () => ({ ChannelSidebar: () => null }));
vi.mock("../chat/MembersPanel", () => ({ MembersPanel: () => null }));
vi.mock("../chat/MembersDialog", () => ({ MembersDialog: () => null }));
vi.mock("../offline/OfflineBanner", () => ({ OfflineBanner: () => null }));
vi.mock("./LeftRail", () => ({ LeftRail: () => null }));

function Channel() {
  const shell = useShell();
  return (
    <div>
      <p>channel:{shell.selectedChannelId}</p>
      <Link to="/settings">Settings</Link>
      <button onClick={shell.showSidebar}>Back</button>
      <button onClick={() => shell.openThread("root")}>Open thread</button>
      <button onClick={() => shell.setMembersOpen(true)}>Open members</button>
      <p>members:{shell.membersOpen ? "open" : "closed"}</p>
      {shell.threadRootId ? (
        <div>
          <p>thread:{shell.threadRootId}</p>
          <button onClick={shell.closeThread}>Close thread</button>
        </div>
      ) : null}
    </div>
  );
}
function setup(entries: string[]) {
  const root = createRootRoute({
    component: () => (
      <AppShell>
        <Outlet />
      </AppShell>
    ),
  });
  const home = createRoute({
    getParentRoute: () => root,
    path: "/",
    component: () => (
      <Link to="/channels/$channelId" params={{ channelId: "general" }}>
        General
      </Link>
    ),
  });
  const channel = createRoute({
    getParentRoute: () => root,
    path: "/channels/$channelId",
    validateSearch: (search: Record<string, unknown>) => ({
      thread: typeof search.thread === "string" ? search.thread : undefined,
    }),
    component: Channel,
  });
  const settings = createRoute({
    getParentRoute: () => root,
    path: "/settings",
    component: () => <p>Settings page</p>,
  });
  const router = createRouter({
    routeTree: root.addChildren([home, channel, settings]),
    history: createMemoryHistory({ initialEntries: entries }),
  });
  render(<RouterProvider router={router} />);
  return router;
}

describe("shell navigation", () => {
  it("restores channel and thread views with Back and Forward", async () => {
    const router = setup(["/"]);
    await userEvent.click(await screen.findByRole("link", { name: "General" }));
    await userEvent.click(await screen.findByRole("button", { name: "Open thread" }));
    await screen.findByText("thread:root");
    expect(router.state.location.search).toMatchObject({ thread: "root" });
    await userEvent.click(screen.getByRole("link", { name: "Settings" }));
    await screen.findByText("Settings page");
    router.history.back();
    await screen.findByText("thread:root");
    router.history.back();
    await waitFor(() => expect(screen.queryByText("thread:root")).toBeNull());
    expect(screen.getByText("channel:general")).toBeVisible();
    router.history.forward();
    await screen.findByText("thread:root");
    await userEvent.click(screen.getByRole("button", { name: "Close thread" }));
    await waitFor(() => expect(screen.queryByText("thread:root")).toBeNull());
    await userEvent.click(screen.getByRole("button", { name: "Back" }));
    await screen.findByRole("link", { name: "General" });
    expect(router.state.location.pathname).toBe("/");
  });

  it("closes a deep-linked thread without leaving its channel", async () => {
    const router = setup(["/", "/channels/general?thread=root"]);
    await userEvent.click(await screen.findByRole("button", { name: "Open thread" }));
    await userEvent.click(await screen.findByRole("button", { name: "Close thread" }));
    await waitFor(() => expect(screen.queryByText("thread:root")).toBeNull());
    expect(router.state.location.pathname).toBe("/channels/general");
    await userEvent.click(screen.getByRole("button", { name: "Back" }));
    await screen.findByRole("link", { name: "General" });
  });
  it("restores a thread with Forward after the members panel was opened", async () => {
    await page.viewport(1440, 900);
    const router = setup(["/", "/channels/general"]);
    await userEvent.click(await screen.findByRole("button", { name: "Open thread" }));
    await screen.findByText("thread:root");
    router.history.back();
    await waitFor(() => expect(screen.queryByText("thread:root")).toBeNull());
    await userEvent.click(screen.getByRole("button", { name: "Open members" }));
    await screen.findByText("members:open");
    router.history.forward();
    await waitFor(() => {
      expect(screen.getByText("members:closed")).toBeVisible();
      expect(screen.getByText("thread:root")).toBeVisible();
      expect(router.state.location.search).toMatchObject({ thread: "root" });
    });
    await userEvent.click(screen.getByRole("button", { name: "Open members" }));
    await waitFor(() => {
      expect(screen.getByText("members:open")).toBeVisible();
      expect(screen.queryByText("thread:root")).toBeNull();
    });
  });
});
