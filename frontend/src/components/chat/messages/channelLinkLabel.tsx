import { MessageCircleIcon } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { ReactNode } from "react";
import type { ChannelLinkInfo } from "./channelLinkInfo";
import type { LinkedChannel } from "./MfmChannels";

/**
 * チャンネルの表示名を返す関数。通常のチャンネルは `#名前`。DM は名前が内部の識別子なので、一覧と同じ総称を出す。
 * 短いリンクと埋め込みカードで共有する。
 */
export function useChannelLabel() {
  const { t } = useTranslation();
  return (channel: Pick<LinkedChannel, "name" | "isDm">) =>
    channel.isDm ? t("dms.title") : `#${channel.name}`;
}

/**
 * URL をそのまま書いたリンクの代わりに出す、短い表示。行き先が分かったときだけ返し、それ以外は `null`
 * （呼び出し側が URL のまま出す）。
 *
 * - メッセージ: 💬 のアイコンと「○○さん: #チャンネル名」
 * - チャンネル: 「#チャンネル名」
 */
export function useShortChannelLinkLabel(info: ChannelLinkInfo): ReactNode | null {
  const { t } = useTranslation();
  const channelLabel = useChannelLabel();
  if (info.status === "channel") return channelLabel(info.channel);
  if (info.status !== "message") return null;
  return (
    <>
      <MessageCircleIcon
        className="mr-1 inline size-[1em] shrink-0 align-[-0.125em]"
        aria-hidden="true"
      />
      {t("channelLink.inlineMessage", {
        name: info.message.authorName,
        channel: channelLabel(info.channel),
      })}
    </>
  );
}
