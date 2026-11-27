// @vitest-environment happy-dom
import { createElement } from "react";
import { act, cleanup, render, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vite-plus/test";
import { toast } from "sonner";

import { ApiError } from "../../../lib/api/client";
import type { MessageReactionRow, MessageRow } from "../../../lib/electric/types";
import { LocaleProvider } from "../../i18n/LocaleProvider";
import { useChannelReactions, type UseChannelReactionsOptions } from "./useChannelReactions";

const mocks = vi.hoisted(() => ({
  reactionsData: [] as MessageReactionRow[],
  reactionsLoading: false,
  reactionsError: false,
  collectionReactions: [] as MessageReactionRow[],
  messagesMap: new Map<string, Partial<MessageRow>>(),
  insert: vi.fn(),
  delete: vi.fn(),
  kitchenEnabled: [] as boolean[],
}));

vi.mock("sonner", () => ({
  toast: { error: vi.fn() },
}));
vi.mock("../../../hooks/useKitchenCatalog", () => ({
  useKitchenCatalog: (enabled: boolean) => {
    mocks.kitchenEnabled.push(enabled);
    return { catalog: null };
  },
}));
vi.mock("../../../hooks/useRealtime", () => ({
  useMessageReactions: () => ({
    data: mocks.reactionsData,
    isLoading: mocks.reactionsLoading,
    isError: mocks.reactionsError,
  }),
}));
vi.mock("../../../lib/electric/collections-context", () => ({
  useCollections: () => ({
    messages: () => ({
      get: (id: string) => mocks.messagesMap.get(id),
    }),
    reactions: () => ({
      values: () => mocks.collectionReactions.values(),
      insert: mocks.insert,
      delete: mocks.delete,
    }),
  }),
}));

const message = (id: string, overrides: Partial<MessageRow> = {}): Partial<MessageRow> => ({
  id,
  channel_id: "c1",
  user_id: "other",
  content: "hello",
  created_at: "2026-01-01T00:00:00Z",
  updated_at: null,
  deleted_at: null,
  parent_id: null,
  thread_archive_at: null,
  ...overrides,
});

const reaction = (
  id: string,
  messageId: string,
  userId: string,
  emoji: string,
): MessageReactionRow => ({
  id,
  message_id: messageId,
  user_id: userId,
  emoji,
  created_at: "2026-01-01T00:00:00Z",
});

function setup(overrides: Partial<UseChannelReactionsOptions> = {}) {
  let latest!: ReturnType<typeof useChannelReactions>;
  function Probe(props: UseChannelReactionsOptions) {
    latest = useChannelReactions(props);
    return null;
  }
  const baseProps: UseChannelReactionsOptions = {
    channelId: "c1",
    currentUserId: "me",
    users: [{ id: "me", display_name: "Alice", user_name: "alice" }],
    usersLoading: false,
    ...overrides,
  };
  const view = render(
    createElement(LocaleProvider, {
      children: createElement(Probe, baseProps),
    }),
  );
  return {
    get current() {
      return latest;
    },
    rerender: (next: Partial<UseChannelReactionsOptions> = {}) =>
      view.rerender(
        createElement(LocaleProvider, {
          children: createElement(Probe, { ...baseProps, ...next }),
        }),
      ),
    unmount: view.unmount,
  };
}

afterEach(() => {
  cleanup();
  localStorage.clear();
  mocks.reactionsData = [];
  mocks.reactionsLoading = false;
  mocks.reactionsError = false;
  mocks.collectionReactions = [];
  mocks.messagesMap = new Map();
  mocks.kitchenEnabled = [];
  mocks.insert.mockReset();
  mocks.delete.mockReset();
  vi.mocked(toast.error).mockReset();
});

describe("useChannelReactions", () => {
  it("adds a reaction and toggles off an existing reaction (normalizing kitchen keys)", async () => {
    mocks.messagesMap.set("m1", message("m1"));
    mocks.insert.mockReturnValue({ when: () => Promise.resolve() });
    mocks.delete.mockReturnValue({ when: () => Promise.resolve() });
    const hook = setup();

    expect(hook.current.canReact).toBe(true);
    await act(async () => {
      await hook.current.handleReaction("m1", "👍");
    });
    expect(mocks.insert).toHaveBeenCalledWith(
      expect.objectContaining({
        message_id: "m1",
        user_id: "me",
        emoji: "👍",
      }),
    );

    mocks.collectionReactions = [reaction("r-kitchen", "m1", "me", "kitchen:1f431_1f600")];
    await act(async () => {
      await hook.current.handleReaction("m1", "kitchen:1f600_1f431");
    });
    expect(mocks.delete).toHaveBeenCalledWith("r-kitchen");
  });

  it("prevents duplicate concurrent writes for the same message and normalized emoji", async () => {
    mocks.messagesMap.set("m1", message("m1"));
    let settle!: () => void;
    mocks.insert.mockReturnValue({ when: () => Promise.resolve() });
    mocks.insert.mockImplementationOnce(() => ({
      when: () =>
        new Promise<void>((resolve) => {
          settle = resolve;
        }),
    }));
    const hook = setup();

    let first!: Promise<void>;
    let second!: Promise<void>;
    await act(async () => {
      first = hook.current.handleReaction("m1", "kitchen:1f600_1f431");
      second = hook.current.handleReaction("m1", "kitchen:1f431_1f600");
      await Promise.resolve();
    });
    expect(mocks.insert).toHaveBeenCalledTimes(1);

    await act(async () => {
      settle();
      await Promise.all([first, second]);
    });

    await act(async () => {
      await hook.current.handleReaction("m1", "kitchen:1f600_1f431");
    });
    expect(mocks.insert).toHaveBeenCalledTimes(2);
  });

  it.each([
    { label: "readOnly", opts: { readOnly: true }, setupState: () => {} },
    { label: "currentUserId null", opts: { currentUserId: null }, setupState: () => {} },
    {
      label: "reactions loading",
      opts: {},
      setupState: () => {
        mocks.reactionsLoading = true;
      },
    },
    {
      label: "reactions error",
      opts: {},
      setupState: () => {
        mocks.reactionsError = true;
      },
    },
  ])("disables reacting and ignores writes when $label", async ({ opts, setupState }) => {
    mocks.messagesMap.set("m1", message("m1"));
    setupState();
    const hook = setup(opts);

    expect(hook.current.canReact).toBe(false);
    act(() => {
      hook.current.openReactionPicker("m1");
    });
    expect(hook.current.reactionMessageId).toBeNull();

    await act(async () => {
      await hook.current.handleReaction("m1", "👍");
    });
    expect(mocks.insert).not.toHaveBeenCalled();
  });

  it("blocks deleted messages and archived threads, and auto-closes open pickers", async () => {
    mocks.messagesMap.set(
      "m-deleted",
      message("m-deleted", { deleted_at: "2026-01-01T01:00:00Z" }),
    );
    mocks.messagesMap.set(
      "root-archived",
      message("root-archived", { thread_archive_at: "2026-01-02T00:00:00Z" }),
    );
    mocks.messagesMap.set(
      "reply-archived",
      message("reply-archived", { parent_id: "root-archived" }),
    );
    mocks.messagesMap.set("m-active", message("m-active"));

    const hook = setup();
    act(() => {
      hook.current.openReactionPicker("m-deleted");
      hook.current.openReactionPicker("reply-archived");
    });
    expect(hook.current.reactionMessageId).toBeNull();

    await act(async () => {
      await hook.current.handleReaction("m-deleted", "👍");
      await hook.current.handleReaction("reply-archived", "👍");
    });
    expect(mocks.insert).not.toHaveBeenCalled();

    act(() => {
      hook.current.openReactionPicker("m-active");
    });
    expect(hook.current.reactionMessageId).toBe("m-active");

    hook.rerender({ readOnly: true });
    await waitFor(() => expect(hook.current.reactionMessageId).toBeNull());

    hook.rerender({ readOnly: false });
    act(() => {
      hook.current.openReactionPicker("m-active");
    });
    expect(hook.current.reactionMessageId).toBe("m-active");

    mocks.messagesMap.set(
      "m-active",
      message("m-active", { thread_archive_at: "2026-01-03T00:00:00Z" }),
    );
    hook.rerender({ readOnly: false });
    await waitFor(() => expect(hook.current.reactionMessageId).toBeNull());
  });

  it("closes an open picker when channelId changes", async () => {
    mocks.messagesMap.set("m1", message("m1"));
    const hook = setup({ channelId: "c1" });
    act(() => {
      hook.current.openReactionPicker("m1");
    });
    expect(hook.current.reactionMessageId).toBe("m1");

    hook.rerender({ channelId: "c2" });
    await waitFor(() => expect(hook.current.reactionMessageId).toBeNull());
  });

  it("shows a localized error toast when write fails without runWrite (even after unmount)", async () => {
    localStorage.setItem("zoo-locale", "ja");
    mocks.messagesMap.set("m1", message("m1"));
    let rejectSettled!: (err: unknown) => void;
    mocks.insert.mockImplementation(() => ({
      when: () =>
        new Promise<void>((_, reject) => {
          rejectSettled = reject;
        }),
    }));

    const hook = setup();
    let pending!: Promise<void>;
    await act(async () => {
      pending = hook.current.handleReaction("m1", "👍");
      await Promise.resolve();
    });
    hook.unmount();

    await act(async () => {
      rejectSettled(new ApiError(500, "Internal Server Error"));
      await pending;
    });

    expect(toast.error).toHaveBeenCalledWith(
      "操作に失敗しました",
      expect.objectContaining({
        id: "message-write-error",
        description: expect.stringContaining("サーバー"),
      }),
    );
  });
});
