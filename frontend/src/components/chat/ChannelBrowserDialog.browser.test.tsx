import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  createRootRoute,
  createRoute,
  createRouter,
  createMemoryHistory,
  RouterProvider,
  Outlet,
} from "@tanstack/react-router";
import { expect, it, vi } from "vite-plus/test";
import { useState } from "react";
import { LocaleProvider } from "../i18n/LocaleProvider";
import type { ChannelRow } from "../../lib/electric/types";
import { ChannelBrowserDialog } from "./ChannelBrowserDialog";

const base: ChannelRow = {
  id: "public",
  channel_name: "public",
  description: "Anyone can read",
  is_private: false,
  is_dm: false,
  created_user_id: "creator",
  created_at: "2026-01-01 00:00:00.000",
  updated_at: null,
  archive_at: null,
  deleted_at: null,
};
it("opens public channel messages without joining and keeps private channels out of discovery", async () => {
  const changed = vi.fn();
  function Home() {
    const [open, setOpen] = useState(true);
    return (
      <ChannelBrowserDialog
        open={open}
        onOpenChange={(value) => {
          changed(value);
          setOpen(value);
        }}
        channels={[
          base,
          { ...base, id: "private", channel_name: "private", is_private: true },
          {
            ...base,
            id: "archived",
            channel_name: "archived",
            archive_at: "2026-01-02 00:00:00.000",
          },
        ]}
        joinedChannelIds={new Set()}
      />
    );
  }
  const root = createRootRoute({ component: Outlet });
  const home = createRoute({ getParentRoute: () => root, path: "/", component: Home });
  const channel = createRoute({
    getParentRoute: () => root,
    path: "/channels/$channelId",
    component: () => <p>Channel messages</p>,
  });
  const router = createRouter({
    routeTree: root.addChildren([home, channel]),
    history: createMemoryHistory({ initialEntries: ["/"] }),
  });
  render(
    <LocaleProvider>
      <RouterProvider router={router} />
    </LocaleProvider>,
  );
  const link = await screen.findByRole("link", { name: /public/ });
  expect(screen.queryByRole("link", { name: /private/ })).toBeNull();
  expect(screen.getByRole("link", { name: /archived/ })).toBeTruthy();
  expect(screen.queryByRole("button", { name: "参加する" })).toBeNull();
  await userEvent.click(link);
  await screen.findByText("Channel messages");
  expect(router.state.location.pathname).toBe("/channels/public");
  expect(changed).toHaveBeenCalledWith(false);
});
