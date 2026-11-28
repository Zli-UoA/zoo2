import { StrictMode } from "react";
import { act, render, screen, waitFor } from "@testing-library/react";
import { expect, it, vi } from "vite-plus/test";
import { EmojiHistoryProvider, useRecentEmojis, useRecordEmoji } from "./history";

const server = vi.hoisted(() => ({
  account: "alice",
  config: {},
  values: new Map<string, string[]>(),
}));
vi.mock("../../hooks/useCurrentUser", () => ({
  useCurrentUser: () => ({ currentUserId: server.account }),
}));
vi.mock("../electric/collections-context", () => ({
  useCollections: () => ({ config: server.config }),
}));
vi.mock("../api/client", () => ({
  createConfiguredApiClient: () => ({
    get: async () => server.values.get(server.account) ?? [],
    post: async (_path: string, { emoji }: { emoji: string }) => {
      const data = [emoji, ...(server.values.get(server.account) ?? []).filter((v) => v !== emoji)];
      server.values.set(server.account, data);
      return { data };
    },
  }),
}));
function Picker() {
  const values = useRecentEmojis();
  const record = useRecordEmoji();
  return <button onClick={() => record("🎉")}>{values.join(",") || "empty"}</button>;
}
it("loads the same account on a fresh device and drops the previous account on switch", async () => {
  const tree = () => (
    <StrictMode>
      <EmojiHistoryProvider>
        <Picker />
      </EmojiHistoryProvider>
    </StrictMode>
  );
  const first = render(tree());
  await act(async () => screen.getByRole("button").click());
  await waitFor(() => expect(server.values.get("alice")).toEqual(["🎉"]));
  first.unmount();
  const second = render(tree());
  expect(await screen.findByText("🎉")).toBeTruthy();
  server.account = "bob";
  second.rerender(tree());
  expect(screen.getByText("empty")).toBeTruthy();
  expect(server.values.has("bob")).toBe(false);
});
