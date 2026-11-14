import { describe, expect, it } from "vite-plus/test";

import {
  ATTACHMENT_SCHEMA,
  CHANNEL_LEAVE_SCHEMA,
  MESSAGE_REACTION_SCHEMA,
  SHAPE_META,
  coerceElectricRow,
  coerceElectricValue,
  toNumber,
} from "./schema";
import type { MessageRow } from "./types";

describe("toNumber", () => {
  it("passes through numbers and parses numeric strings", () => {
    expect(toNumber(7)).toBe(7);
    expect(toNumber("7")).toBe(7);
  });
});

describe("coerceElectricValue", () => {
  it("converts numeric-as-string values to numbers", () => {
    expect(coerceElectricValue("int2", "1")).toBe(1);
    expect(coerceElectricValue("int4", "42")).toBe(42);
    expect(coerceElectricValue("float4", "1.5")).toBe(1.5);
    expect(coerceElectricValue("float8", "1.5")).toBe(1.5);
  });

  it("leaves int8 and numeric as strings to avoid precision loss", () => {
    expect(coerceElectricValue("int8", "9007199254740993")).toBe("9007199254740993");
    expect(coerceElectricValue("numeric", "0.25")).toBe("0.25");
  });

  it("keeps timestamp values as strings", () => {
    expect(coerceElectricValue("timestamp", "2026-10-15 13:32:55.123")).toBe(
      "2026-10-15 13:32:55.123",
    );
  });

  it("parses boolean strings so that 'false' is not truthy", () => {
    expect(coerceElectricValue("bool", "true")).toBe(true);
    expect(coerceElectricValue("bool", "t")).toBe(true);
    expect(coerceElectricValue("bool", "false")).toBe(false);
    expect(coerceElectricValue("bool", "f")).toBe(false);
  });

  it("passes through null, undefined, and non-numeric values", () => {
    expect(coerceElectricValue("int4", null)).toBeNull();
    expect(coerceElectricValue("int4", undefined)).toBeUndefined();
    expect(coerceElectricValue("bool", true)).toBe(true);
    expect(coerceElectricValue("text", "abc")).toBe("abc");
  });
});

describe("coerceElectricRow", () => {
  it("preserves ISO timestamps and null optional columns", () => {
    const row: Record<string, unknown> = {
      id: "message-1",
      user_id: "user-1",
      channel_id: "channel-1",
      content: "hello",
      created_at: "2026-10-15T13:32:55.123",
      updated_at: null,
      deleted_at: null,
    };

    const coerced: MessageRow = coerceElectricRow("messages", row);
    expect(coerced.created_at).toBe("2026-10-15T13:32:55.123");
    expect(coerced.updated_at).toBeNull();
    expect(coerced.deleted_at).toBeNull();
  });

  it("does not mutate the input row", () => {
    const row = { id: "message-1", created_at: "2026-10-15T13:32:55.123" };
    coerceElectricRow("messages", row);
    expect(row).toEqual({ id: "message-1", created_at: "2026-10-15T13:32:55.123" });
  });
});

describe("SHAPE_META", () => {
  it("maps the channel user proxy path to the channel_user table", () => {
    expect(SHAPE_META.channelUsers).toEqual({
      table: "channel_user",
      proxyPath: "channel_users",
    });
  });

  it("maps the channel leave proxy path to the channel_leave table", () => {
    expect(SHAPE_META.channelLeaves).toEqual({
      table: "channel_leave",
      proxyPath: "channel_leaves",
    });
    expect(CHANNEL_LEAVE_SCHEMA.columns.left_at).toBe("timestamp");
  });

  it("maps the attachments shape to the attachments table", () => {
    expect(SHAPE_META.attachments).toEqual({
      table: "attachments",
      proxyPath: "attachments",
    });
    expect(ATTACHMENT_SCHEMA.columns.file_size).toBe("int8");
  });

  it("maps the reaction shape to the message_reactions table", () => {
    expect(SHAPE_META.messageReactions).toEqual({
      table: "message_reactions",
      proxyPath: "message_reactions",
    });
  });
});

describe("reaction rows", () => {
  it("keeps timestamps as strings when coercing message_reactions", () => {
    expect(
      coerceElectricRow("message_reactions", {
        id: "reaction-1",
        message_id: "message-1",
        user_id: "user-1",
        emoji: "👍",
        created_at: "2026-10-15 13:32:55.123",
      }),
    ).toEqual({
      id: "reaction-1",
      message_id: "message-1",
      user_id: "user-1",
      emoji: "👍",
      created_at: "2026-10-15 13:32:55.123",
    });
    expect(MESSAGE_REACTION_SCHEMA.columns.created_at).toBe("timestamp");
  });
});

it("coerces notification read status so false remains unread", () => {
  expect(coerceElectricRow("notifications", { id: "n1", is_read: "false" }).is_read).toBe(false);
  expect(coerceElectricRow("notifications", { id: "n1", is_read: "t" }).is_read).toBe(true);
});

describe("call_invites", () => {
  it("coerces the invite row and keeps timestamptz as a string", () => {
    const row = coerceElectricRow("call_invites", {
      id: "00000000-0000-4000-8000-000000000001",
      channel_id: "c1",
      inviter_id: "alice",
      invitee_id: "bob",
      status: "pending",
      created_at: "2026-11-14 10:00:00.000+00",
      expires_at: "2026-11-14 10:00:30.000+00",
    });
    expect(row.expires_at).toBe("2026-11-14 10:00:30.000+00");
    expect(row.status).toBe("pending");
  });
});
