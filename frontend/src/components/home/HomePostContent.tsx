import { useTranslation } from "react-i18next";
import { ActivityPubContent, type CustomEmoji } from "./ActivityPubEmoji";

/** The same MFM, emoji and link rendering in Home and public profiles. */
export function HomePostContent({ content, emojis }: { content: string; emojis?: CustomEmoji[] }) {
  const { t } = useTranslation();
  return (
    <div className="mfm-body mt-2 whitespace-pre-wrap break-words text-sm leading-relaxed [overflow-wrap:anywhere]">
      <ActivityPubContent
        text={content.trim() ? content : t("home.attachmentsOnly")}
        emojis={emojis}
      />
    </div>
  );
}
