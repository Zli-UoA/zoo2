import { hasCardBlocks } from "../AppBlocks";
import { hidesContent, parseComponents } from "../components/parse";
import { parseMessageCards, type CardData } from "./MessageCard";

/**
 * 投稿の本文（`content`）の代わりに、何を描くか。`MessageBody` と、メッセージリンクの埋め込みが、同じ
 * 判定を使う。
 *
 * - `hidden`: 部品（Components v2）か、App のカードのブロックを、本文の代わりに描く。`content` は出さない。
 * - `cards`: 連携カード（`card`）を描く。`content` は出さない。
 * - `text`: `content` を MFM として描く。
 */
export type MessageBodySource =
  | { kind: "hidden" }
  | { kind: "cards"; cards: CardData[] }
  | { kind: "text" };

export function messageBodySource(input: {
  components?: string | null;
  appBlocks?: string | null;
  card?: string | null;
}): MessageBodySource {
  // 部品は MessageAppContent が描画する。本文を隠すフラグがあれば、本文は出さない
  // 読めない部品（`{}` など）は、ないものとして旧形式の表示に戻す（MessageAppContent と同じ判定）
  const doc = parseComponents(input.components);
  if (hidesContent(doc)) return { kind: "hidden" };
  if (!doc && hasCardBlocks(input.appBlocks)) return { kind: "hidden" };
  const cards = doc ? null : parseMessageCards(input.card);
  return cards ? { kind: "cards", cards } : { kind: "text" };
}
