import { SearchIcon } from "lucide-react";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import type { ThreadParticipant } from "../../../hooks/useThreadMembership";
import { Button } from "../../ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "../../ui/dialog";
import { Input } from "../../ui/input";
import { MemberRow } from "../MemberRow";
import {
  filterThreadParticipants,
  participantLabel,
  sortThreadParticipants,
} from "./threadParticipants";

/** 自分の参加状態。`null` は、メンバーでない・読み取り専用など、参加・退出の操作を出さない場合。 */
export type ThreadParticipantsStatus = "joined" | "notJoined" | "checking" | null;

export interface ThreadParticipantsAction {
  kind: "leave" | "join" | "reopenAndJoin";
  onClick: () => void;
  disabled: boolean;
}

/**
 * スレッドのメンバー。ほかのポップアウト（チャンネルを探す・設定）と同じ、画面の中央に出すダイアログで、
 * 中身はチャンネルのメンバー（`ChannelMembersDialog`）にそろえる。スレ主（親メッセージの投稿者）を先頭に固定して
 * 「スレ主」の印を、自分に「あなた」の印を付け、名前で絞り込め、参加・退出の操作を出す。
 *
 * 一覧は、サーバーが返す先頭から最大50人。絞り込みは、その範囲に対して行う。
 */
export function ThreadParticipantsDialog({
  open,
  onOpenChange,
  count,
  participants,
  creatorId,
  currentUserId,
  status,
  action,
  membershipError = false,
  onMembershipRetry,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** 参加中の人数（一覧に載る人数より多いことがある）。まだ読み込めていないときは `undefined`。 */
  count: number | undefined;
  participants: readonly ThreadParticipant[];
  /** スレ主（親メッセージの投稿者）。 */
  creatorId: string | null | undefined;
  currentUserId: string | null | undefined;
  status: ThreadParticipantsStatus;
  action: ThreadParticipantsAction | null;
  /** 参加・退出に失敗した。ダイアログが開いているあいだは、背面のペインの表示が見えず、読み上げられない。 */
  membershipError?: boolean;
  onMembershipRetry?: () => void;
}) {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const sorted = useMemo(
    () => sortThreadParticipants(participants, creatorId),
    [participants, creatorId],
  );
  const visible = useMemo(() => filterThreadParticipants(sorted, query), [sorted, query]);
  const hidden = count === undefined ? 0 : count - participants.length;

  function handleOpenChange(next: boolean) {
    if (!next) setQuery("");
    onOpenChange(next);
  }

  const actionLabel =
    action?.kind === "leave"
      ? t("threads.leave")
      : action?.kind === "reopenAndJoin"
        ? t("threads.reopenAndJoin")
        : t("threads.join");

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            {t("members.title")}
            <span className="rounded bg-muted px-2 py-0.5 text-xs font-normal tabular-nums text-muted-foreground">
              {count ?? "…"}
            </span>
          </DialogTitle>
          <DialogDescription>{t("threads.participantsDescription")}</DialogDescription>
        </DialogHeader>

        <div className="relative">
          <SearchIcon
            aria-hidden="true"
            className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground"
          />
          <Input
            type="search"
            className="w-full pl-8 [&::-webkit-search-cancel-button]:appearance-none"
            placeholder={t("threads.participantsSearch")}
            aria-label={t("threads.participantsSearch")}
            value={query}
            onChange={(event) => setQuery(event.target.value)}
          />
        </div>

        <div>
          <ul
            aria-label={
              count === undefined ? t("members.title") : t("threads.participants", { count })
            }
            className="flex max-h-[min(20rem,50dvh)] min-h-0 flex-col gap-0.5 overflow-y-auto"
          >
            {visible.map((participant) => (
              <MemberRow
                key={participant.id}
                label={participantLabel(participant)}
                avatarUrl={participant.avatar_url}
                creatorTag={participant.id === creatorId ? t("threads.threadStarter") : undefined}
                isMe={participant.id === currentUserId}
              />
            ))}
          </ul>
          {/* 検索結果の変化（0 件・先頭の範囲だけの絞り込み）は、常設のライブリージョンで伝える。
              入力欄のフォーカスは動かさない。空のあいだは余白も出さない。 */}
          <div
            role="status"
            className="flex flex-col gap-1 text-xs text-muted-foreground [&:not(:empty)]:mt-4"
          >
            {participants.length === 0 ? (
              <p>{t("threads.participantsEmpty")}</p>
            ) : visible.length === 0 ? (
              <p>{t("members.usersEmpty")}</p>
            ) : null}
            {hidden > 0 ? (
              <p>
                {query.trim()
                  ? // 絞り込みは、読み込んだ先頭の範囲だけ。ほかの人は、検索に出ない。
                    t("threads.participantsFilterPartial", {
                      loaded: participants.length,
                      count: hidden,
                    })
                  : t("threads.participantsMore", { count: hidden })}
              </p>
            ) : null}
          </div>
        </div>
        {membershipError ? (
          <p role="alert" className="text-xs text-destructive">
            {t("threads.membershipError")}{" "}
            {onMembershipRetry ? (
              <Button variant="ghost" size="sm" onClick={onMembershipRetry}>
                {t("offline.retry")}
              </Button>
            ) : null}
          </p>
        ) : null}

        {status ? (
          <div className="flex items-center justify-between gap-2 border-t pt-3">
            {/* 参加・退出の成功で文言が変わる。フォーカスを動かさずに、変化を通知する（検索結果の領域とは別）。 */}
            <span role="status" className="text-xs text-muted-foreground">
              {status === "joined"
                ? t("members.joined")
                : status === "notJoined"
                  ? t("members.notJoined")
                  : t("members.checking")}
            </span>
            {action ? (
              <Button
                size="sm"
                variant={action.kind === "leave" ? "outline" : "default"}
                disabled={action.disabled}
                onClick={action.onClick}
              >
                {actionLabel}
              </Button>
            ) : null}
          </div>
        ) : null}
      </DialogContent>
    </Dialog>
  );
}
