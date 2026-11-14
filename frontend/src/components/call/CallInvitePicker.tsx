import { useState } from "react";
import { useTranslation } from "react-i18next";

import { AppAvatar } from "../chat/AppIdentity";
import { Button } from "../ui/button";

export interface CallInviteCandidate {
  id: string;
  name: string;
  avatarUrl?: string | null;
}

export interface CallInvitePickerProps {
  /** 通話に入っていない、呼べるメンバー。 */
  candidates: readonly CallInviteCandidate[];
  /** あと何人まで呼べるか（通話の定員 - 在室 - 呼び出し中）。 */
  slots: number;
  busy?: boolean;
  error?: string | null;
  onInvite: (userIds: string[]) => void;
}

/** 通話中に、チャンネルのメンバーを選んで呼ぶ。 */
export function CallInvitePicker({
  candidates,
  slots,
  busy = false,
  error = null,
  onInvite,
}: CallInvitePickerProps) {
  const { t } = useTranslation();
  const [selected, setSelected] = useState<ReadonlySet<string>>(new Set());
  // 呼べなくなった人（通話に入った・退出した）は、選択から外す。
  const chosen = candidates.filter((candidate) => selected.has(candidate.id));
  const full = slots <= 0;

  if (candidates.length === 0) {
    return <p className="text-sm text-muted-foreground">{t("call.inviteNobody")}</p>;
  }
  return (
    <form
      className="space-y-3"
      onSubmit={(event) => {
        event.preventDefault();
        if (chosen.length > 0 && !busy) onInvite(chosen.map((candidate) => candidate.id));
      }}
    >
      {full ? <p className="text-xs text-muted-foreground">{t("call.inviteFull")}</p> : null}
      <ul className="max-h-56 space-y-1 overflow-y-auto">
        {candidates.map((candidate) => {
          const checked = selected.has(candidate.id);
          return (
            <li key={candidate.id}>
              <label className="flex cursor-pointer items-center gap-2 rounded-md px-1.5 py-1 hover:bg-muted">
                <input
                  type="checkbox"
                  className="size-4 accent-emerald-600"
                  checked={checked}
                  disabled={busy || (!checked && chosen.length >= slots)}
                  onChange={(event) =>
                    setSelected((current) => {
                      const next = new Set(current);
                      if (event.target.checked) next.add(candidate.id);
                      else next.delete(candidate.id);
                      return next;
                    })
                  }
                />
                <AppAvatar label={candidate.name} src={candidate.avatarUrl} className="size-6" />
                <span className="min-w-0 truncate text-sm">{candidate.name}</span>
              </label>
            </li>
          );
        })}
      </ul>
      {error ? (
        <p role="alert" className="text-xs text-destructive">
          {error}
        </p>
      ) : null}
      <Button type="submit" size="sm" className="w-full" disabled={busy || chosen.length === 0}>
        {t("call.inviteSubmit", { count: chosen.length })}
      </Button>
    </form>
  );
}
