import type { ThreadParticipant } from "../../../hooks/useThreadMembership";

/** メンバー表示に使うラベル。表示名 → ユーザー名の順にフォールバックする。 */
export function participantLabel(
  participant: Pick<ThreadParticipant, "display_name" | "user_name">,
) {
  return participant.display_name || participant.user_name;
}

/**
 * スレ主（親メッセージの投稿者）を先頭に固定し、残りは表示名のABC順（チャンネルのメンバー一覧と同じ）。
 * 同名はユーザー名・IDで安定させる。
 */
export function sortThreadParticipants(
  participants: readonly ThreadParticipant[],
  creatorId: string | null | undefined,
): ThreadParticipant[] {
  const collator = new Intl.Collator("en", { sensitivity: "base", numeric: true });
  return [...participants].sort((a, b) => {
    const creatorOrder = Number(b.id === creatorId) - Number(a.id === creatorId);
    if (creatorOrder) return creatorOrder;
    return (
      collator.compare(participantLabel(a), participantLabel(b)) ||
      collator.compare(a.user_name, b.user_name) ||
      a.id.localeCompare(b.id)
    );
  });
}

/** 表示名・ユーザー名の部分一致（大文字小文字を無視）で絞り込む。空の検索語は全員を返す。 */
export function filterThreadParticipants(
  participants: readonly ThreadParticipant[],
  query: string,
): ThreadParticipant[] {
  const needle = query.trim().toLowerCase();
  if (!needle) return [...participants];
  return participants.filter(
    (participant) =>
      participantLabel(participant).toLowerCase().includes(needle) ||
      participant.user_name.toLowerCase().includes(needle),
  );
}
