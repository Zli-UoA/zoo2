import "../../../styles.css";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vite-plus/test";

import {
  ForwardedMessagesProvider,
  type ForwardEntryState,
} from "../../../hooks/useForwardedMessages";
import type { ResolvedForwardMessage } from "../../../lib/api/types";
import { LocaleProvider } from "../../i18n/LocaleProvider";
import { ForwardedMessageCard } from "./ForwardedMessageCard";

function resolvedSample(overrides: Partial<ResolvedForwardMessage> = {}): ResolvedForwardMessage {
  return {
    id: "fwd-1",
    channel_id: "ch-src",
    channel_name: "general",
    is_private: false,
    is_dm: false,
    dm_participants: [],
    parent_id: null,
    user_id: "user-1",
    created_at: "2026-01-02 10:20:00.000",
    updated_at: null,
    content: "転送元の投稿本文です",
    card: null,
    components: null,
    message_kind: "message",
    author: {
      id: "user-1",
      user_name: "alice",
      display_name: "Alice",
      avatar_url: null,
      is_bot: false,
      app_id: null,
    },
    emojis: {},
    attachments: [],
    has_forwards: false,
    ...overrides,
  };
}

function renderCard(
  messageId: string,
  entries: Map<string, ForwardEntryState>,
  onRetry = vi.fn(),
  currentUserId: string | null = "user-viewer",
) {
  return {
    onRetry,
    ...render(
      <LocaleProvider>
        <ForwardedMessagesProvider value={{ entries, retry: onRetry, currentUserId }}>
          <ForwardedMessageCard messageId={messageId} />
        </ForwardedMessagesProvider>
      </LocaleProvider>,
    ),
  };
}

describe("ForwardedMessageCard", () => {
  beforeEach(() => {
    localStorage.setItem("zoo-locale", "ja");
  });

  afterEach(() => {
    localStorage.clear();
  });

  it("renders resolved public channel forward with link to source message", () => {
    const entries = new Map<string, ForwardEntryState>([
      ["fwd-1", { status: "ok", ...resolvedSample() }],
    ]);
    renderCard("fwd-1", entries);

    expect(screen.getByText("Alice")).toBeVisible();
    expect(screen.getByText("転送元の投稿本文です")).toBeVisible();
    const link = screen.getByRole("link", { name: "#general への投稿" });
    expect(link).toHaveAttribute("href", expect.stringContaining("/channels/ch-src?message=fwd-1"));
  });

  it("renders DM label and thread reply link when forwarding a DM thread reply", () => {
    const entries = new Map<string, ForwardEntryState>([
      [
        "fwd-dm",
        {
          status: "ok",
          ...resolvedSample({
            id: "fwd-dm",
            channel_id: "dm-1",
            channel_name: "dm:user-2:user-viewer",
            is_private: true,
            is_dm: true,
            dm_participants: [
              { id: "user-viewer", user_name: "viewer", display_name: "Viewer" },
              { id: "user-2", user_name: "bob", display_name: "Bob" },
            ],
            parent_id: "thread-root-1",
          }),
        },
      ],
    ]);
    renderCard("fwd-dm", entries);

    const link = screen.getByRole("link", { name: /Bob.*のスレッドへの返信/ });
    expect(link).toHaveAttribute(
      "href",
      expect.stringContaining("/channels/dm-1?thread=thread-root-1&message=fwd-dm"),
    );
  });

  it("renders compact nesting note link without recursive resolution when has_forwards is true", () => {
    const entries = new Map<string, ForwardEntryState>([
      ["fwd-nest", { status: "ok", ...resolvedSample({ id: "fwd-nest", has_forwards: true }) }],
    ]);
    renderCard("fwd-nest", entries);

    const nestedLink = screen.getByRole("link", {
      name: "この投稿にも転送（引用）があります · 元の投稿を見る",
    });
    expect(nestedLink).toHaveAttribute(
      "href",
      expect.stringContaining("/channels/ch-src?message=fwd-nest"),
    );
  });

  it("renders loading, deleted, unavailable, and retryable error states", async () => {
    const entries = new Map<string, ForwardEntryState>([
      ["m-load", { status: "loading" }],
      ["m-del", { status: "deleted" }],
      ["m-unavail", { status: "unavailable" }],
      ["m-err", { status: "error" }],
    ]);
    const retry = vi.fn();
    render(
      <LocaleProvider>
        <ForwardedMessagesProvider value={{ entries, retry, currentUserId: "u1" }}>
          <ForwardedMessageCard messageId="m-load" />
          <ForwardedMessageCard messageId="m-del" />
          <ForwardedMessageCard messageId="m-unavail" />
          <ForwardedMessageCard messageId="m-err" />
        </ForwardedMessagesProvider>
      </LocaleProvider>,
    );

    expect(screen.getByLabelText("転送されたメッセージを読み込み中")).toBeInTheDocument();
    expect(screen.getByText("このメッセージは削除されました")).toBeVisible();
    expect(screen.getByText("このメッセージは閲覧権限がないため表示できません")).toBeVisible();

    await userEvent.click(screen.getByRole("button", { name: "再試行" }));
    expect(retry).toHaveBeenCalledWith("m-err");
  });
});
