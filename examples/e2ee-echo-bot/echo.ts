import { createHash } from "node:crypto";

import type { AppEvent } from "../../sdk/src/index.ts";
import type { DecryptedMessage, ZooE2ee } from "../../sdk/src/e2ee.ts";

/** The reply to a decrypted message, or `null` to stay silent. */
export function replyFor(message: DecryptedMessage, botUserId: string): string | null {
  // Act only on messages whose sending device is verified (signed by a trusted master key).
  if (!message.senderVerified || message.userId === botUserId) return null;
  const command = message.body.match(/^!echo\s+([\s\S]+)$/);
  if (command) return `echo: ${command[1]!.trim()}`;
  if (message.mentions.user_ids.includes(botUserId)) return `echo: ${message.body}`;
  return null;
}

/**
 * Handle one event: decrypt messages of encrypted channels and answer `!echo <text>` or a mention in
 * the message's thread, encrypted. Plaintext channels are left to other bots. Returns the reply id.
 */
export async function handleEvent(e2ee: ZooE2ee, event: AppEvent): Promise<string | null> {
  if (event.type !== "message.created") return null;
  const message = await e2ee.decryptEvent(event);
  if (!message) return null;
  const reply = replyFor(message, e2ee.userId);
  if (reply === null) return null;
  // The same id for every delivery of this event: if a reply was posted but the event was not
  // acknowledged, the redelivery posts nothing new (Zoo returns the existing message for that id).
  return e2ee.sendMessage(message.channelId, reply, { parentId: message.parentId ?? message.id, id: replyId(event.id) });
}

/** A UUID (version 4 layout) derived from the event id, stable across redeliveries. */
export function replyId(eventId: string): string {
  const bytes = createHash("sha256").update(`e2ee-echo-bot:reply:${eventId}`).digest().subarray(0, 16);
  bytes[6] = (bytes[6]! & 0x0f) | 0x40;
  bytes[8] = (bytes[8]! & 0x3f) | 0x80;
  const hex = bytes.toString("hex");
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}
