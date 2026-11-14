import type { UserRow } from "../../../lib/electric/types";

// Keep boundaries and username characters aligned with backend/src/notifications.rs.
export const MENTION_PATTERN = /(^|[\s([{<（「『"'。、])(@[a-zA-Z0-9._-]+)/g;

export function mentionQuery(value: string, caret: number) {
  const match = /(?:^|[\s([{<（「『"'。、])@([\p{L}\p{M}\p{N}._-]*)$/u.exec(value.slice(0, caret));
  return match
    ? { start: caret - match[1]!.length - 1, end: caret, query: match[1]!.normalize("NFKC") }
    : null;
}

export interface MentionOption {
  name: string;
  label: string;
}
export function mentionOptions(
  users: UserRow[],
  memberIds: ReadonlySet<string>,
  query: string,
): MentionOption[] {
  const search = query.normalize("NFKC").toLowerCase();
  return [
    { name: "channel", label: "@channel" },
    { name: "here", label: "@here" },
    ...users
      .filter(
        (user) =>
          memberIds.has(user.id) &&
          // Slack から取り込んだ未連携のプレースホルダなど、無効化済みのユーザーは選ばせない
          !user.is_deactivated &&
          /^[a-zA-Z0-9._-]+$/.test(user.user_name) &&
          !["channel", "here"].includes(user.user_name.toLowerCase()),
      )
      .map((user) => ({ name: user.user_name, label: user.display_name || user.user_name })),
  ]
    .filter(
      (option) =>
        option.name.normalize("NFKC").toLowerCase().includes(search) ||
        option.label.normalize("NFKC").toLowerCase().includes(search),
    )
    .slice(0, 8);
}

export function insertMention(value: string, range: { start: number; end: number }, name: string) {
  const inserted = `@${name} `;
  return {
    value: value.slice(0, range.start) + inserted + value.slice(range.end),
    caret: range.start + inserted.length,
  };
}

const MENTION_PUNCTUATION = /[.,!?:;。、！？]+$/u;

/**
 * 暗号化チャンネルの送信に付ける平文の `mentions`（docs/e2ee.md の「メッセージ」）。サーバーは本文を
 * 読めないので、平文チャンネルでサーバーが本文から作るのと同じ規則（`backend/src/notifications.rs` の
 * `extract_mentions`）で、送信側が作る。ユーザー名は、語のまま、なければ末尾の句読点を外して引く。
 */
export function encryptedMentions(
  content: string,
  users: readonly Pick<UserRow, "id" | "user_name">[],
): { user_ids: string[]; channel: boolean; here: boolean } {
  const byName = new Map(users.map((user) => [user.user_name, user.id]));
  const userIds = new Set<string>();
  let channel = false;
  let here = false;
  for (const match of content.matchAll(MENTION_PATTERN)) {
    const raw = match[2]!.slice(1);
    const trimmed = raw.replace(MENTION_PUNCTUATION, "");
    if (trimmed.toLowerCase() === "channel") channel = true;
    else if (trimmed.toLowerCase() === "here") here = true;
    else {
      const id = byName.get(raw) ?? byName.get(trimmed);
      if (id) userIds.add(id);
    }
  }
  return { user_ids: [...userIds].sort(), channel, here };
}
