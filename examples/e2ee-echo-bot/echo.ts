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
  return e2ee.sendMessage(message.channelId, reply, { parentId: message.parentId ?? message.id });
}
