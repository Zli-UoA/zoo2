import { useEffect, useMemo, useState } from "react";

import { createConfiguredApiClient } from "../lib/api/client";
import { useCollections } from "../lib/electric/collections-context";
import type { ChannelRow } from "../lib/electric/types";
import { type MemberUnreadableReason, unreadableMembers } from "../lib/e2ee/memberReadability";
import { fetchRecipients } from "../lib/e2ee/megolmApi";

const EMPTY = new Map<string, MemberUnreadableReason>();

/**
 * 暗号化チャンネルで、新しいメッセージを読めないメンバー（暗号化を設定していない、使える端末がない）。
 *
 * メンバー欄を開いているあいだだけ、配布先のスナップショットを 1 回取る。メンバー、端末の削除、
 * クロス署名鍵が変わると `membership_epoch` が増えるので、そのときだけ取り直す（ポーリングはしない）。
 * 暗号化していないチャンネルと、取れなかったとき（オフライン、権限の変化など）は何も出さない。
 */
export function useUnreadableMembers(
  channel: Pick<ChannelRow, "id" | "encrypted" | "membership_epoch">,
): Map<string, MemberUnreadableReason> {
  const { config } = useCollections();
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  const [state, setState] = useState<{
    key: string;
    members: Map<string, MemberUnreadableReason>;
  } | null>(null);
  const key = `${channel.id}:${channel.membership_epoch ?? ""}`;

  useEffect(() => {
    if (!channel.encrypted) return;
    const controller = new AbortController();
    fetchRecipients(client, channel.id, null, controller.signal)
      .then((pages) => {
        if (!controller.signal.aborted) setState({ key, members: unreadableMembers(pages) });
      })
      .catch(() => {
        if (!controller.signal.aborted) setState({ key, members: EMPTY });
      });
    return () => controller.abort();
  }, [channel.encrypted, channel.id, client, key]);

  if (!channel.encrypted || state?.key !== key) return EMPTY;
  return state.members;
}
