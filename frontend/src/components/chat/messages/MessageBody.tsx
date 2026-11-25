import { useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { PencilIcon } from "lucide-react";

import type { DisplayEmoji } from "../../../lib/electric/types";
import { normalizeUtcTimestamp } from "../../../lib/timestamps";
import { useLocale } from "../../i18n/LocaleProvider";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "../../ui/tooltip";
import { LinkPreviews } from "./LinkPreviews.tsx";
import { MfmContent } from "./MfmContent";
import { MessageCard } from "./MessageCard";
import { messageBodySource } from "./messageBodySource";
import { type MentionAllowlist } from "./mentions";
import { fullTimestamp } from "./messageFormat";

export function EditedIndicator({ at }: { at: string }) {
  const tooltipId = useId();
  const [tooltipOpen, setTooltipOpen] = useState(false);
  const { t } = useTranslation();
  const { locale } = useLocale();
  return (
    <TooltipProvider>
      <Tooltip onOpenChange={setTooltipOpen}>
        <TooltipTrigger
          render={<span role="img" tabIndex={0} />}
          aria-label={t("messages.edited")}
          aria-describedby={tooltipOpen ? tooltipId : undefined}
          className="ml-1 inline-flex rounded-sm align-middle text-muted-foreground/80 outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
        >
          <PencilIcon className="size-3" strokeWidth={1.5} aria-hidden="true" />
        </TooltipTrigger>
        <TooltipContent id={tooltipId} role="tooltip">
          {t("messages.edited")} ·{" "}
          <time dateTime={normalizeUtcTimestamp(at)}>{fullTimestamp(at, locale)}</time>
        </TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}

/** 通常投稿は MFM、連携カードは専用の描画を使う。 */
export function MessageBody({
  content,
  card: data,
  appBlocks,
  editedAt,
  emojis,
  previews = "auto",
  mentions,
  components,
}: {
  emojis?: Record<string, DisplayEmoji>;
  editedAt?: string | null;
  content: string;
  card?: string | null;
  appBlocks?: string | null;
  /** メッセージ部品（`messages.components`）。フラグが立っていれば本文の代わりに部品だけを表示する */
  components?: string | null;
  /** `manual` は、押したときだけ URL カードを読み込む（暗号化チャンネル）。`none` はカードを描画しない。 */
  previews?: "auto" | "manual" | "none";
  /** 暗号化メッセージで、メンションとして表示してよい `@name`（{@link MfmContent} の `mentions`）。 */
  mentions?: MentionAllowlist;
}) {
  // 本文の代わりに部品・カードを描く投稿かは、メッセージリンクの埋め込みと同じ判定で決める。
  const source = messageBodySource({ components, appBlocks, card: data });
  if (source.kind === "hidden") return null;
  if (source.kind === "cards")
    return (
      <>
        {source.cards.map((card, index) => (
          <MessageCard key={index} card={card} />
        ))}
        {editedAt ? <EditedIndicator at={editedAt} /> : null}
      </>
    );
  return (
    <>
      <div className="mfm-body whitespace-pre-wrap break-words text-sm leading-relaxed">
        <MfmContent content={content} emojis={emojis} mentions={mentions} />
        {editedAt ? <EditedIndicator at={editedAt} /> : null}
      </div>
      {/* 画面外の行もマウントする画面（スレッドなど）があるので、画面の近くに来るまで取得しない（`defer`）。
          暗号化チャンネル（`manual`）では、外部の URL のカードを押したときだけ取得する（docs/e2ee.md の
          「機能ごとの扱い」）。このアプリのチャンネル・メッセージへのリンクは、外へ送らないので、押さずに埋め込みにする。 */}
      {previews === "none" ? null : (
        <LinkPreviews defer content={content} manualExternal={previews === "manual"} />
      )}
    </>
  );
}
