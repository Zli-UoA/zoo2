import type {
  AttachmentRow,
  ChannelRow,
  ChannelUserRow,
  MessageReactionRow,
  MessageRow,
  NotificationRow,
  TableName,
  TableRowMap,
  UserRow,
} from "./types";

export type ElectricType =
  | "bool"
  | "int2"
  | "int4"
  | "int8"
  | "float4"
  | "float8"
  | "numeric"
  | "text"
  | "varchar"
  | "uuid"
  | "timestamp"
  | "timestamptz"
  | "date"
  | "time"
  | "jsonb";

export const NUMERIC_ELECTRIC_TYPES: ReadonlySet<ElectricType> = new Set<ElectricType>([
  "int2",
  "int4",
  "float4",
  "float8",
]);

export type ShapeName =
  | "channels"
  | "messages"
  | "messageReactions"
  | "users"
  | "channelUsers"
  | "notifications"
  | "attachments";

export interface ShapeMeta {
  table: TableName;
  proxyPath: string;
}

export const SHAPE_META: Record<ShapeName, ShapeMeta> = {
  notifications: { table: "notifications", proxyPath: "notifications" },
  channels: { table: "channels", proxyPath: "channels" },
  messages: { table: "messages", proxyPath: "messages" },
  messageReactions: { table: "message_reactions", proxyPath: "message_reactions" },
  users: { table: "users", proxyPath: "users" },
  channelUsers: { table: "channel_user", proxyPath: "channel_users" },
  attachments: { table: "attachments", proxyPath: "attachments" },
};

export interface ShapeSchema<Row> {
  table: TableName;
  columns: Record<keyof Row & string, ElectricType>;
}

export const USER_SCHEMA: ShapeSchema<UserRow> = {
  table: "users",
  columns: {
    id: "text",
    user_name: "text",
    display_name: "varchar",
    is_bot: "bool",
    app_id: "text",
    avatar_url: "text",
    bot_mentionable: "bool",
    created_at: "timestamp",
    updated_at: "timestamp",
  },
};

export const CHANNEL_SCHEMA: ShapeSchema<ChannelRow> = {
  table: "channels",
  columns: {
    id: "text",
    channel_name: "varchar",
    description: "varchar",
    created_user_id: "text",
    is_private: "bool",
    is_dm: "bool",
    created_at: "timestamp",
    updated_at: "timestamp",
    archive_at: "timestamp",
    deleted_at: "timestamp",
  },
};

export const MESSAGE_SCHEMA: ShapeSchema<MessageRow> = {
  table: "messages",
  columns: {
    id: "text",
    user_id: "text",
    channel_id: "text",
    content: "text",
    card: "text",
    app_blocks: "text",
    app_attachments: "text",
    app_unfurls: "text",
    app_origin_id: "text",
    created_at: "timestamp",
    updated_at: "timestamp",
    deleted_at: "timestamp",
    parent_id: "text",
  },
};

export const MESSAGE_REACTION_SCHEMA: ShapeSchema<MessageReactionRow> = {
  table: "message_reactions",
  columns: {
    id: "text",
    message_id: "text",
    user_id: "text",
    emoji: "text",
    created_at: "timestamp",
  },
};

export const CHANNEL_USER_SCHEMA: ShapeSchema<ChannelUserRow> = {
  table: "channel_user",
  columns: {
    user_id: "text",
    channel_id: "text",
    joined_at: "timestamp",
  },
};

export const NOTIFICATION_SCHEMA: ShapeSchema<NotificationRow> = {
  table: "notifications",
  columns: {
    id: "text",
    user_id: "text",
    actor_id: "text",
    channel_id: "text",
    message_id: "text",
    notification_type: "text",
    is_read: "bool",
    created_at: "timestamp",
  },
};

export const ATTACHMENT_SCHEMA: ShapeSchema<AttachmentRow> = {
  table: "attachments",
  columns: {
    id: "text",
    message_id: "text",
    uploader_id: "text",
    channel_id: "text",
    file_name: "text",
    file_size: "int8",
    content_type: "text",
    storage_key: "text",
    created_at: "timestamp",
  },
};

export const SHAPE_SCHEMAS: { [K in TableName]: ShapeSchema<TableRowMap[K]> } = {
  notifications: NOTIFICATION_SCHEMA,
  users: USER_SCHEMA,
  channels: CHANNEL_SCHEMA,
  messages: MESSAGE_SCHEMA,
  message_reactions: MESSAGE_REACTION_SCHEMA,
  channel_user: CHANNEL_USER_SCHEMA,
  attachments: ATTACHMENT_SCHEMA,
};

export function toNumber(value: number | string): number {
  return typeof value === "number" ? value : Number(value);
}

export function coerceElectricValue(type: ElectricType, value: unknown): unknown {
  if (value === null || value === undefined) {
    return value;
  }
  if (NUMERIC_ELECTRIC_TYPES.has(type) && typeof value === "string") {
    return toNumber(value);
  }
  // Electric は boolean も文字列（"true" / "false"、稀に "t" / "f"）で配信する。
  // 文字列のまま扱うと "false" が truthy になり、真偽の判定を誤る。
  if (type === "bool" && typeof value === "string") {
    if (value === "true" || value === "t") {
      return true;
    }
    if (value === "false" || value === "f") {
      return false;
    }
  }
  return value;
}

export function coerceElectricRow<K extends TableName>(
  table: K,
  row: Record<string, unknown>,
): TableRowMap[K] {
  const schema = SHAPE_SCHEMAS[table];
  const coerced: Record<string, unknown> = { ...row };
  const columns = Object.entries(schema.columns) as [string, ElectricType][];
  for (const [column, type] of columns) {
    if (column in coerced) {
      coerced[column] = coerceElectricValue(type, coerced[column]);
    }
  }
  return coerced as unknown as TableRowMap[K];
}
