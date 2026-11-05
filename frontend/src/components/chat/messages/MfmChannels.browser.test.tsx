import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  RouterProvider,
  useRouterState,
} from "@tanstack/react-router";
import { expect, it } from "vite-plus/test";
import { MfmChannelsProvider } from "./MfmChannels";
import { MfmContent } from "./MfmContent";

it("navigates hashtags and history without document navigation or losing shell state", async () => {
  function Shell() {
    const path = useRouterState({ select: (state) => state.location.pathname });
    return (
      <MfmChannelsProvider
        channels={[
          { id: "general", channel_name: "general", deleted_at: null },
          { id: "development", channel_name: "開発", deleted_at: null },
        ]}
      >
        <input aria-label="Draft" defaultValue="" />
        <output aria-label="Current route">{path}</output>
        <MfmContent content="#general $[jelly #開発] #unknown" />
      </MfmChannelsProvider>
    );
  }
  const root = createRootRoute({ component: Shell });
  const channel = createRoute({ getParentRoute: () => root, path: "/channels/$channelId" });
  const history = createMemoryHistory({ initialEntries: ["/channels/general"] });
  const router = createRouter({ routeTree: root.addChildren([channel]), history });
  render(<RouterProvider router={router} />);

  const draft = await screen.findByRole("textbox", { name: "Draft" });
  await userEvent.type(draft, "unsent message");
  const link = screen.getByRole("link", { name: "#開発" });
  expect(link.getAttribute("href")).toBe("/channels/development");
  expect(screen.queryByRole("link", { name: "#unknown" })).toBeNull();

  // The browser's default anchor navigation must be cancelled by the router.
  const prevented: boolean[] = [];
  const observeClick = (event: MouseEvent) => prevented.push(event.defaultPrevented);
  document.addEventListener("click", observeClick);
  try {
    await userEvent.click(link);
  } finally {
    document.removeEventListener("click", observeClick);
  }
  expect(prevented).toEqual([true]);
  await waitFor(() =>
    expect(screen.getByLabelText("Current route").textContent).toBe("/channels/development"),
  );

  history.back();
  await waitFor(() =>
    expect(screen.getByLabelText("Current route").textContent).toBe("/channels/general"),
  );
  history.forward();
  await waitFor(() =>
    expect(screen.getByLabelText("Current route").textContent).toBe("/channels/development"),
  );
  expect(screen.getByRole("textbox", { name: "Draft" })).toBe(draft);
  expect((draft as HTMLInputElement).value).toBe("unsent message");

  screen.getByRole("link", { name: "#general" }).focus();
  await userEvent.keyboard("{Enter}");
  await waitFor(() =>
    expect(screen.getByLabelText("Current route").textContent).toBe("/channels/general"),
  );
});
