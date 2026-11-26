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
 * メンバー欄を開いているあいだだけ、配布先のスナップショットを取る。取り直すのは次のときだけで、
 * ポーリングはしない。
 * - メンバー、端末の削除、クロス署名鍵が変わったとき（`membership_epoch` が増える）
 * - タブが前面に戻ったとき。端末の登録や解錠（自己署名鍵での署名）は `membership_epoch` を増やさず、
 *   Web は端末の Shape も購読していないので、相手が端末を足したあとは、ここで反映する。
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
  const [visibleTick, setVisibleTick] = useState(0);

  useEffect(() => {
    if (!channel.encrypted || typeof document === "undefined") return;
    const onVisible = () => {
      if (document.visibilityState === "visible") setVisibleTick((tick) => tick + 1);
    };
    document.addEventListener("visibilitychange", onVisible);
    return () => document.removeEventListener("visibilitychange", onVisible);
  }, [channel.encrypted]);

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
  }, [channel.encrypted, channel.id, client, key, visibleTick]);

  if (!channel.encrypted || state?.key !== key) return EMPTY;
  return state.members;
}
