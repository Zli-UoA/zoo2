import { createCollection, localOnlyCollectionOptions } from "@tanstack/react-db";
import { render, screen, waitFor } from "@testing-library/react";
import { expect, it, vi } from "vite-plus/test";
import type { MessageRow, MessageReactionRow, UserRow } from "../lib/electric/types";
import { useThreadFeed, useThreadRoot } from "./useThread";

const access = vi.hoisted(() => ({ messages: vi.fn(), reactions: vi.fn() }));
const users = createCollection(
  localOnlyCollectionOptions<UserRow>({ getKey: (row) => row.id, initialData: [] }),
);
vi.mock("../lib/electric/collections-context", () => ({
  useCollections: () => ({ ...access, users }),
}));
vi.mock("../lib/emoji/workspace", () => ({ useWorkspaceEmojis: () => ({ emojis: {} }) }));

it("does not access message or reaction collections while the thread is closed", async () => {
  const messages = createCollection(
    localOnlyCollectionOptions<MessageRow>({ getKey: (row) => row.id, initialData: [] }),
  );
  const reactions = createCollection(
    localOnlyCollectionOptions<MessageReactionRow>({ getKey: (row) => row.id, initialData: [] }),
  );
  access.messages.mockReturnValue(messages);
  access.reactions.mockReturnValue(reactions);
  function Probe({ root }: { root: string | null }) {
    const result = useThreadFeed("c1", root);
    useThreadRoot("c1", root);
    return <p data-testid="count">{result.data?.length ?? 0}</p>;
  }
  const view = render(<Probe root={null} />);
  await expect.element(screen.getByTestId("count")).toHaveTextContent("0");
  expect(access.messages).not.toHaveBeenCalled();
  expect(access.reactions).not.toHaveBeenCalled();
  view.rerender(<Probe root="root" />);
  await waitFor(() => expect(access.messages).toHaveBeenCalledWith("c1"));
  await waitFor(() => expect(access.reactions).toHaveBeenCalledWith("c1"));
  view.unmount();
  await Promise.all([messages.cleanup(), reactions.cleanup(), users.cleanup()]);
});
