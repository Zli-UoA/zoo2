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

/**
 * 暗号化メッセージで、メンションとして表示してよい `@name`。復号した `mentions` から作る
 * （{@link decryptedMentionAllowlist}）。本文に `@name` があっても、ここにないものはメンションとして
 * 表示しない（docs/e2ee.md の「Megolm の平文」と「機能ごとの扱い」のメンション通知）。
 */
export interface MentionAllowlist {
  /** メンションしたユーザーのユーザー名。 */
  userNames: ReadonlySet<string>;
  channel: boolean;
  here: boolean;
}

/**
 * 復号した `mentions` を、本文の `@name` と突き合わせられる形にする。`userNames` はユーザー ID から
 * ユーザー名を引く表で、引けない ID（ユーザー一覧にまだない、削除された）は表示に使わない。
 */
export function decryptedMentionAllowlist(
  mentions: { user_ids: readonly string[]; channel: boolean; here: boolean },
  userNames: ReadonlyMap<string, string>,
): MentionAllowlist {
  const names = new Set<string>();
  for (const id of mentions.user_ids) {
    const name = userNames.get(id);
    if (name) names.add(name);
  }
  return { userNames: names, channel: mentions.channel, here: mentions.here };
}

/**
 * 本文の `@name`（先頭の `@` を含む）が、`allowlist` のメンションに当たるか。送信側の
 * {@link encryptedMentions} と同じく、`@channel` と `@here` は大文字と小文字を区別せず、ユーザー名は
 * 語のまま、なければ末尾の句読点を外して引く。
 */
export function isAllowedMention(token: string, allowlist: MentionAllowlist): boolean {
  const raw = token.startsWith("@") ? token.slice(1) : token;
  const trimmed = raw.replace(MENTION_PUNCTUATION, "");
  const lower = trimmed.toLowerCase();
  if (lower === "channel") return allowlist.channel;
  if (lower === "here") return allowlist.here;
  return allowlist.userNames.has(raw) || allowlist.userNames.has(trimmed);
}
