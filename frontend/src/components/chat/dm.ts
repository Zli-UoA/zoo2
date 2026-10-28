import type { ChannelUserRow, UserRow } from "../../lib/electric/types";
import { DEFAULT_LOCALE, INTL_LOCALES, type Locale } from "../../lib/i18n/locale";
import { userLabel } from "./memberCandidates";

export const DM_MAX_MEMBERS = 10;
export const DM_MAX_TARGETS = DM_MAX_MEMBERS - 1;
export const DM_VISIBLE_NAMES = 3;

const GROUP_DM_PREFIX = "dm:group:";

export function isGroupDm(channelName: string): boolean {
  return channelName.startsWith(GROUP_DM_PREFIX);
}

export function parseOneToOneDmIds(channelName: string): [string, string] | null {
  if (isGroupDm(channelName)) {
    return null;
  }
  const match = /^dm:([^:]+):([^:]+)$/.exec(channelName);
  if (!match) {
    return null;
  }
  return [match[1], match[2]];
}

export interface DmDisplayNameInput {
  channelName: string;
  members: Pick<ChannelUserRow, "user_id">[];
  users: Pick<UserRow, "id" | "display_name" | "user_name">[];
  currentUserId: string | null;
  locale?: Locale;
  maxNames?: number;
}

export interface DmDisplayName {
  names: string[];
  remaining: number;
}

export function resolveDmDisplayName({
  channelName,
  members,
  users,
  currentUserId,
  locale = DEFAULT_LOCALE,
  maxNames = DM_VISIBLE_NAMES,
}: DmDisplayNameInput): DmDisplayName {
  const labels = new Map(users.map((user) => [user.id, userLabel(user)]));
  const oneToOneIds = parseOneToOneDmIds(channelName);
  const participantIds = oneToOneIds ?? members.map((member) => member.user_id);
  const otherIds = currentUserId
    ? participantIds.filter((userId) => userId !== currentUserId)
    : participantIds;
  const collator = new Intl.Collator(INTL_LOCALES[locale]);
  const names = otherIds
    .map((userId) => labels.get(userId))
    .filter((label): label is string => Boolean(label))
    .sort((a, b) => collator.compare(a, b));
  const limit = Math.max(1, maxNames);
  return {
    names: names.slice(0, limit),
    remaining: Math.max(0, names.length - limit),
  };
}

export interface DmCandidateFilter<T extends Pick<UserRow, "id" | "display_name" | "user_name">> {
  users: T[];
  currentUserId: string | null;
  selectedIds: Iterable<string>;
  query: string;
}

export function filterDmCandidates<T extends Pick<UserRow, "id" | "display_name" | "user_name">>({
  users,
  currentUserId,
  selectedIds,
  query,
}: DmCandidateFilter<T>): T[] {
  const selected = new Set(selectedIds);
  const needle = query.trim().toLowerCase();
  return users
    .filter((user) => user.id !== currentUserId && !selected.has(user.id))
    .filter((user) => {
      if (!needle) {
        return true;
      }
      return (
        userLabel(user).toLowerCase().includes(needle) ||
        user.user_name.toLowerCase().includes(needle)
      );
    });
}
