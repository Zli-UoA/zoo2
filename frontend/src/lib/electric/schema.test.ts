import { describe, expect, it } from "vite-plus/test";

import { SHAPE_META, coerceElectricRow, coerceElectricValue, toNumber } from "./schema";
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
    expect(coerceElectricValue("int8", "1000000")).toBe(1000000);
    expect(coerceElectricValue("float8", "1.5")).toBe(1.5);
    expect(coerceElectricValue("numeric", "0.25")).toBe(0.25);
  });

  it("keeps timestamp values as ISO strings", () => {
    expect(coerceElectricValue("timestamp", "2026-10-15T13:32:55.123")).toBe(
      "2026-10-15T13:32:55.123",
    );
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
});
