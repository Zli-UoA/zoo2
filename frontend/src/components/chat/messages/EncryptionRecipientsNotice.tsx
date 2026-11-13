import { useTranslation } from "react-i18next";
import { useEffect, useRef, useState } from "react";
import { LockKeyholeIcon, ShieldAlertIcon } from "lucide-react";

import type { UserRow } from "../../../lib/electric/types";
import type { RecipientSummary, UndeliverableReason } from "../../../lib/e2ee/store/megolm";
import { userLabel } from "../memberCandidates";

const REASON_KEYS = {
  notSetUp: "e2ee.undeliverable.notSetUp",
  invalidKeys: "e2ee.undeliverable.invalidKeys",
  needsReverification: "e2ee.undeliverable.needsReverification",
  pinPending: "e2ee.undeliverable.pinPending",
  blocked: "e2ee.undeliverable.blocked",
} as const satisfies Record<UndeliverableReason, string>;

/**
 * 暗号化チャンネルの送信画面の表示（docs/e2ee.md の「鍵の配布先の判定」）。
 *
 * - 今のセッションの配布先（ユーザーと端末の数）。
 * - 鍵を配れない相手と理由（暗号化を設定していない、再検証が必要など）。配れなくても送信は止めない。
 * - 前回の送信から配布先のユーザーが増えたら、その人。メンバーの一覧はサーバーが決めるもので
 *   暗号で認証されていないので、利用者が気付けるようにする。
 */
export function EncryptionRecipientsNotice({
  summary,
  users,
}: {
  summary: RecipientSummary;
  users: readonly Pick<UserRow, "id" | "user_name" | "display_name">[];
}) {
  const { t } = useTranslation();
  const previous = useRef<RecipientSummary | null>(null);
  const [added, setAdded] = useState<string[]>([]);
  useEffect(() => {
    const before = previous.current;
    if (before && before !== summary) {
      const known = new Set(before.userIds);
      setAdded(summary.userIds.filter((id) => !known.has(id)));
    }
    previous.current = summary;
  }, [summary]);
  const byId = new Map(users.map((user) => [user.id, user]));
  const name = (id: string) => {
    const user = byId.get(id);
    return user ? userLabel(user) : id;
  };
  return (
    <div
      role="status"
      aria-live="polite"
      className="mx-3 mb-2 flex flex-col gap-1 rounded-lg border bg-muted/40 px-3 py-2 text-xs text-muted-foreground"
    >
      <p className="flex items-center gap-1.5">
        <LockKeyholeIcon className="size-3.5 shrink-0" aria-hidden="true" />
        {t("e2ee.recipients.summary", {
          users: summary.userCount,
          devices: summary.deviceCount,
        })}
      </p>
      {added.length > 0 ? (
        <p className="text-foreground">
          {t("e2ee.recipients.added", { names: added.map(name).join(", ") })}
        </p>
      ) : null}
      {summary.undeliverable.length > 0 ? (
        <div className="flex flex-col gap-0.5 text-amber-800 dark:text-amber-300">
          <p className="flex items-center gap-1.5 font-medium">
            <ShieldAlertIcon className="size-3.5 shrink-0" aria-hidden="true" />
            {t("e2ee.undeliverable.title")}
          </p>
          <ul className="ml-5 list-disc">
            {summary.undeliverable.map((entry) => (
              <li key={entry.userId}>
                {t(REASON_KEYS[entry.reason], { name: name(entry.userId) })}
              </li>
            ))}
          </ul>
        </div>
      ) : null}
    </div>
  );
}
