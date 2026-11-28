import { useTranslation } from "react-i18next";
import { LockKeyholeIcon, ShieldAlertIcon } from "lucide-react";

import type { UserRow } from "../../../lib/electric/types";
import type { RecipientSummary, UndeliverableReason } from "../../../lib/e2ee/store/megolm";
import { userLabel } from "../memberCandidates";

// 辞書のキーは `e2ee.undeliverable.` + 接尾辞。x18n が前方一致で追えるようにする。
const REASON_SUFFIXES = {
  notSetUp: "notSetUp",
  invalidKeys: "invalidKeys",
  needsReverification: "needsReverification",
  pinPending: "pinPending",
  blocked: "blocked",
} as const satisfies Record<UndeliverableReason, string>;

/**
 * 暗号化チャンネルの送信画面の表示（docs/e2ee.md の「鍵の配布先の判定」）。気付いてほしいことがあるときだけ出す。
 * 何もないときは要素ごと出さないので、読み上げの live region（`role="status"`）は呼び出し側の常に置く親が持つ。
 * 配布先の数（何人の何台の端末か）は、毎回同じで読み流されるので出さない。
 *
 * - 鍵を配れない相手と理由（暗号化を設定していない、再検証が必要など）。配れなくても送信は止めない。
 * - 前回判定した配布先からユーザーが増えたら、その人（暗号ストアに記録した前回のメンバーと比べる
 *   ので、チャンネルを切り替えても開き直しても出る）。メンバーの一覧はサーバーが決めるもので
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
  const added = summary.addedUserIds ?? [];
  if (added.length === 0 && summary.undeliverable.length === 0) return null;
  const byId = new Map(users.map((user) => [user.id, user]));
  const name = (id: string) => {
    const user = byId.get(id);
    return user ? userLabel(user) : id;
  };
  return (
    <div className="mx-3 mb-2 flex flex-col gap-1 rounded-lg border bg-muted/40 px-3 py-2 text-xs text-muted-foreground">
      {added.length > 0 ? (
        <p className="flex items-center gap-1.5 text-foreground">
          <LockKeyholeIcon className="size-3.5 shrink-0" aria-hidden="true" />
          {t("e2ee.recipients.added", { names: added.map(name).join(", ") })}
        </p>
      ) : null}
      {summary.undeliverable.length > 0 ? (
        <div className="flex flex-col gap-0.5 text-warning-strong">
          <p className="flex items-center gap-1.5 font-medium">
            <ShieldAlertIcon className="size-3.5 shrink-0" aria-hidden="true" />
            {t("e2ee.undeliverable.title")}
          </p>
          <ul className="ml-5 list-disc">
            {summary.undeliverable.map((entry) => (
              <li key={entry.userId}>
                {t(`e2ee.undeliverable.${REASON_SUFFIXES[entry.reason]}`, {
                  name: name(entry.userId),
                })}
              </li>
            ))}
          </ul>
        </div>
      ) : null}
    </div>
  );
}
