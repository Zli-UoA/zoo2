import { describe, expect, it, vi } from "vite-plus/test";

import type { ApiClient, ApiEnvelope } from "./client";
import {
  activeNotesCursor,
  channelNoteExpiresAt,
  channelNoteTitle,
  createChannelNotesApi,
  deletedNotesCursor,
  deletedNotesPath,
  formatRestoreDeadline,
  isUnknownCreateOutcome,
  moveNote,
  notesPath,
  parseDeletedAtUtc,
  trimNoteTitleWhiteSpace,
  validateNoteDraft,
  type ChannelNoteSummary,
  type DeletedChannelNoteSummary,
} from "./channelNotes";

function note(overrides: Partial<ChannelNoteSummary> = {}): ChannelNoteSummary {
  return {
    id: "note-1",
    channel_id: "channel-1",
    title: "Runbook",
    position: 3,
    version: 2,
    created_user_id: "alice",
    created_at: "2026-11-28T01:02:03.000",
    updated_at: "2026-11-28T01:02:03.000",
    order_token: "token",
    ...overrides,
  };
}

function deleted(overrides: Partial<DeletedChannelNoteSummary> = {}): DeletedChannelNoteSummary {
  return {
    ...note(),
    deleted_at: "2026-11-28T01:02:03.456",
    ...overrides,
  };
}

describe("paths and cursors", () => {
  it("builds the active and deleted note paths", () => {
    expect(notesPath("c 1")).toBe("/api/channels/c%201/notes");
    expect(deletedNotesPath("c 1")).toBe("/api/channels/c%201/notes/deleted");
  });

  it("keeps the server cursor components in their original order", () => {
    expect(activeNotesCursor(note({ id: "note 1", position: 7, order_token: "abc" }))).toBe(
      "abc 7 note 1",
    );
    expect(activeNotesCursor(note({ id: "", position: 0, order_token: "abc" }))).toBe("abc 0 ");
    expect(
      deletedNotesCursor(deleted({ id: "note 1", deleted_at: "2026-11-28T01:02:03.456" })),
    ).toBe("2026-11-28T01:02:03.456 note 1");
  });
});

describe("deleted note deadline", () => {
  it("parses the API timestamp as UTC", () => {
    expect(parseDeletedAtUtc("2026-11-28T01:02:03.456").toISOString()).toBe(
      "2026-11-28T01:02:03.456Z",
    );
    expect(channelNoteExpiresAt(deleted()).toISOString()).toBe("2026-11-29T01:02:03.456Z");
  });

  it("rejects timestamps that the API does not produce", () => {
    expect(() => parseDeletedAtUtc("2026-11-28T01:02:03Z")).toThrow();
    expect(() => parseDeletedAtUtc("2026-99-28T01:02:03.456")).toThrow();
  });

  it("formats the restore deadline in UTC without shifting the value", () => {
    expect(formatRestoreDeadline("2026-11-28T23:30:00.000")).toBe("2026/11/29 23:30 UTC");
  });
});

describe("moveNote", () => {
  const items = ["a", "b", "c", "d"];
  it("moves an item down and up while preserving the others", () => {
    expect(moveNote(items, 0, 2)).toEqual(["b", "c", "a", "d"]);
    expect(moveNote(items, 3, 1)).toEqual(["a", "d", "b", "c"]);
    expect(moveNote(items, 1, 1)).toBe(items);
  });

  it("ignores out-of-range moves", () => {
    expect(moveNote(items, -1, 2)).toBe(items);
    expect(moveNote(items, 0, 4)).toBe(items);
  });
});

describe("validateNoteDraft", () => {
  it("accepts a title trimmed to the API limit and content at the limit", () => {
    expect(validateNoteDraft({ title: "  Runbook  ", content: "x".repeat(100_000) })).toBeNull();
  });

  it("uses the same Unicode White_Space boundaries as Rust trim", () => {
    // Rust は U+0085 を前後から除く（199 文字）。JS の trim だけだと 201 文字になる。
    expect(validateNoteDraft({ title: `\u0085${"x".repeat(199)}\u0085`, content: "" })).toBeNull();
    // Rust は U+FEFF を除かないので、単独でも 1 文字の有効なタイトル。
    expect(validateNoteDraft({ title: "\uFEFF", content: "" })).toBeNull();
    // U+FEFF を JS trim で落とすと 200 文字に見えるが、Rust では 201 文字。
    expect(validateNoteDraft({ title: `${"x".repeat(200)}\uFEFF`, content: "" })).toBe(
      "titleTooLong",
    );
    // U+0085 だけなら Rust では空。JS trim だけだと 1 文字に見える。
    expect(validateNoteDraft({ title: "\u0085", content: "" })).toBe("titleRequired");
  });

  it("reports the first invalid field using translation keys", () => {
    expect(validateNoteDraft({ title: "   ", content: "" })).toBe("titleRequired");
    expect(validateNoteDraft({ title: "x".repeat(201), content: "" })).toBe("titleTooLong");
    expect(validateNoteDraft({ title: "Runbook", content: "x".repeat(100_001) })).toBe(
      "contentTooLong",
    );
    expect(validateNoteDraft({ title: "Runbook", content: "a\0b" })).toBe("nul");
  });

  it("keeps a valid FEFF title in display fallback", () => {
    expect(trimNoteTitleWhiteSpace("\u0085\uFEFF\u0085")).toBe("\uFEFF");
    expect(channelNoteTitle("\uFEFF", "Untitled")).toBe("\uFEFF");
    expect(channelNoteTitle("\u0085", "Untitled")).toBe("Untitled");
  });
});

describe("isUnknownCreateOutcome", () => {
  it("treats network failures, 408, and 5xx as unknown", () => {
    expect(isUnknownCreateOutcome(new TypeError("Failed to fetch"))).toBe(true);
    expect(isUnknownCreateOutcome({ status: 408 })).toBe(true);
    expect(isUnknownCreateOutcome({ status: 500 })).toBe(true);
    expect(isUnknownCreateOutcome({ status: 503 })).toBe(true);
  });

  it("does not mark a definite rejection as unknown", () => {
    expect(isUnknownCreateOutcome({ status: 400 })).toBe(false);
    expect(isUnknownCreateOutcome({ status: 403 })).toBe(false);
    expect(isUnknownCreateOutcome({ status: 409 })).toBe(false);
  });
});

describe("createChannelNotesApi", () => {
  it("passes the page cursor through the query string", async () => {
    const requestWithTx = vi.fn(async () => ({ data: [], txid: null }) as ApiEnvelope<never>);
    const client = { requestWithTx } as unknown as ApiClient;
    const api = createChannelNotesApi(client);

    await api.list("channel-1", { after: "token 3 note 1", limit: 50 });
    expect(requestWithTx).toHaveBeenCalledWith(
      "/api/channels/channel-1/notes?limit=50&after=token%203%20note%201",
      { signal: undefined },
    );

    await api.listDeleted("channel-1", { after: "2026-11-28T01:02:03.456 note 1" });
    expect(requestWithTx).toHaveBeenLastCalledWith(
      "/api/channels/channel-1/notes/deleted?limit=50&after=2026-11-28T01%3A02%3A03.456%20note%201",
      { signal: undefined },
    );
  });

  it("sends versions for updates and deletes and reorders with expected ids", async () => {
    const requestWithTx = vi.fn(async () => ({ data: {}, txid: "1" }) as ApiEnvelope<never>);
    const client = { requestWithTx } as unknown as ApiClient;
    const api = createChannelNotesApi(client);

    await api.update("channel-1", "note 1", { title: "T", content: "C", version: 4 });
    expect(requestWithTx).toHaveBeenLastCalledWith("/api/channels/channel-1/notes/note%201", {
      method: "PATCH",
      body: { title: "T", content: "C", version: 4 },
    });

    await api.remove("channel-1", "note 1", 4);
    expect(requestWithTx).toHaveBeenLastCalledWith("/api/channels/channel-1/notes/note%201", {
      method: "DELETE",
      body: { version: 4 },
    });

    await api.reorder("channel-1", { expectedNoteIds: ["a", "b"], noteIds: ["b", "a"] });
    expect(requestWithTx).toHaveBeenLastCalledWith("/api/channels/channel-1/notes/order", {
      method: "PATCH",
      body: { expected_note_ids: ["a", "b"], note_ids: ["b", "a"] },
    });
  });

  it("creates and restores without a version body", async () => {
    const requestWithTx = vi.fn(async () => ({ data: {}, txid: "1" }) as ApiEnvelope<never>);
    const client = { requestWithTx } as unknown as ApiClient;
    const api = createChannelNotesApi(client);

    await api.create("channel-1", { title: "T", content: "C" });
    expect(requestWithTx).toHaveBeenLastCalledWith("/api/channels/channel-1/notes", {
      method: "POST",
      body: { title: "T", content: "C" },
    });

    await api.restore("channel-1", "note 1");
    expect(requestWithTx).toHaveBeenLastCalledWith(
      "/api/channels/channel-1/notes/note%201/restore",
      { method: "POST" },
    );

    await api.getDeleted("channel-1", "note 1");
    expect(requestWithTx).toHaveBeenLastCalledWith(
      "/api/channels/channel-1/notes/deleted/note%201",
      { method: "GET" },
    );
  });
});
