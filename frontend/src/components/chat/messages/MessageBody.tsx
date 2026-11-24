import { useId, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { PencilIcon } from "lucide-react";

import type { DisplayEmoji } from "../../../lib/electric/types";
import { normalizeUtcTimestamp } from "../../../lib/timestamps";
import { hasCardBlocks } from "../AppBlocks";
import { hidesContent, parseComponents } from "../components/parse";
import { useLocale } from "../../i18n/LocaleProvider";
import { Button } from "../../ui/button";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "../../ui/tooltip";
import { LinkPreviews } from "./LinkPreviews.tsx";
import { MfmContent } from "./MfmContent";
import { MessageCard, parseMessageCards } from "./MessageCard";
import { previewUrls } from "./linkPreviews";
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

/**
 * 暗号化チャンネルの URL カードは、押したときだけ読み込む。カードの取得で、Summaly と画像プロクシの
 * 運用者に URL が見えるため（docs/e2ee.md の「機能ごとの扱い」）。
 */
function ManualLinkPreviews({ content }: { content: string }) {
  const { t } = useTranslation();
  // 押したときの本文にだけ許可する。編集で本文（URL）が変わったら、もう一度押すまで取得しない。
  const [loadedFor, setLoadedFor] = useState<string | null>(null);
  const hasUrls = useMemo(() => previewUrls(content).length > 0, [content]);
  if (!hasUrls) return null;
  if (loadedFor === content) return <LinkPreviews content={content} />;
  return (
    <Button
      type="button"
      variant="link"
      size="sm"
      className="h-auto min-h-8 px-0 text-xs text-muted-foreground"
      onClick={() => setLoadedFor(content)}
    >
      {t("e2ee.loadPreviews")}
    </Button>
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
  // 部品は MessageAppContent が描画する。本文を隠すフラグがあれば、本文は出さない
  // 読めない部品（`{}` など）は、ないものとして旧形式の表示に戻す（MessageAppContent と同じ判定）
  const doc = parseComponents(components);
  if (hidesContent(doc)) return null;
  if (!doc && hasCardBlocks(appBlocks)) return null;
  const cards = doc ? null : parseMessageCards(data);
  if (cards)
    return (
      <>
        {cards.map((card, index) => (
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
      {previews === "manual" ? (
        <ManualLinkPreviews content={content} />
      ) : previews === "auto" ? (
        <LinkPreviews content={content} />
      ) : null}
    </>
  );
}
