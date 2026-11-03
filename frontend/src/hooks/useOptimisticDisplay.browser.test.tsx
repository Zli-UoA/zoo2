import { render, screen, waitFor } from "@testing-library/react";
import { createCollection } from "@tanstack/react-db";
import { expect, it, vi } from "vite-plus/test";
import { useOptimisticDisplay } from "./useOptimisticDisplay";
import type { MessageRow, MessageReactionRow } from "../lib/electric/types";

let sequence = 0;
function controlled<T extends { id: string }>(rows: T[], pending: Promise<void>) {
  return createCollection<T>({
    id: `optimistic-display-${sequence++}`,
    getKey: (row) => row.id,
    sync: {
      sync({ begin, write, commit, markReady }) {
        begin();
        for (const value of rows) write({ type: "insert", value });
        void commit();
        markReady();
      },
    },
    onInsert: () => pending,
    onDelete: () => pending,
  });
}
let messages: ReturnType<typeof controlled<MessageRow>>;
let reactions: ReturnType<typeof controlled<MessageReactionRow>>;
vi.mock("../lib/electric/collections-context", () => ({
  useOptionalCollections: () => null,
  useCollections: () => ({ messages: () => messages, reactions: () => reactions }),
}));

it("renders pending reactions and replies immediately and rolls both back without dropping joined remote rows", async () => {
  let reject!: (error: Error) => void;
  const pending = new Promise<void>((_, fail) => {
    reject = fail;
  });
  const own: MessageReactionRow = {
    id: "own",
    message_id: "root",
    user_id: "me",
    emoji: "👍",
    created_at: "2026-01-01",
  };
  const peer = { ...own, id: "peer", user_id: "peer", user_name: "peer" };
  const root: MessageRow = {
    id: "root",
    channel_id: "c",
    user_id: "me",
    content: "Root",
    parent_id: null,
    created_at: "2026-01-01",
    updated_at: null,
    deleted_at: null,
    display_data: { emojis: {}, attachments: [], reactions: [peer, own], reply_count: 10 },
  };
  messages = controlled([root], pending);
  reactions = controlled([own], pending);
  const data = [root];
  function Probe() {
    const rows = useOptimisticDisplay("c", data);
    const display = rows![0]!.display_data!;
    return (
      <p data-testid="display">
        {display.reply_count} replies, {display.reactions.length} reactions,{" "}
        {display.reactions[0]?.user_name}
      </p>
    );
  }
  const view = render(<Probe />);
  await expect
    .element(screen.getByTestId("display"))
    .toHaveTextContent("10 replies, 2 reactions, peer");
  await Promise.all([messages.preload(), reactions.preload()]);
  const reply = messages.insert({
    ...root,
    id: "reply",
    parent_id: "root",
    display_data: undefined,
  });
  const deleted = reactions.delete("own");
  const writes = [
    reply.isPersisted.promise.catch(() => undefined),
    deleted.isPersisted.promise.catch(() => undefined),
  ];
  await waitFor(() =>
    expect(screen.getByTestId("display")).toHaveTextContent("11 replies, 1 reactions, peer"),
  );
  reject(new Error("Rejected by server"));
  await Promise.all(writes);
  await waitFor(() =>
    expect(screen.getByTestId("display")).toHaveTextContent("10 replies, 2 reactions, peer"),
  );
  view.unmount();
  await Promise.all([messages.cleanup(), reactions.cleanup()]);
});
