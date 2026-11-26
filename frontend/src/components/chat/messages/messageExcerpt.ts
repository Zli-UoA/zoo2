import { parse, toString, type MfmNode } from "mfm-js";

/** 要約の最大文字数（コードポイント単位）。画面側でも行数で切るので、これは保険の上限。 */
export const EXCERPT_MAX_LENGTH = 200;

function plainText(nodes: readonly MfmNode[]): string {
  return nodes.map(nodeText).join("");
}

/**
 * MFM のノードを、装飾やリンクを外した素の文字列にする。
 * リンクの中にリンクを置けない埋め込みカードの本文用で、`<a>` を作らず、表示順の文字だけを返す。
 */
function nodeText(node: MfmNode): string {
  switch (node.type) {
    case "text":
      return node.props.text;
    case "unicodeEmoji":
      return node.props.emoji;
    case "emojiCode":
      return `:${node.props.name}:`;
    case "mention":
      return node.props.acct;
    case "hashtag":
      return `#${node.props.hashtag}`;
    case "url":
      return node.props.url;
    case "link":
      return plainText(node.children);
    case "search":
      return node.props.query;
    case "inlineCode":
      return node.props.code;
    case "blockCode":
      return ` ${node.props.code} `;
    case "mathInline":
    case "mathBlock":
      return node.props.formula;
    case "quote":
    case "center":
      // ブロックの前後に空白を置いて、前後の文と単語がつながらないようにする。
      return ` ${plainText(node.children)} `;
    case "bold":
    case "small":
    case "italic":
    case "strike":
    case "fn":
    case "plain":
      return plainText(node.children);
    default:
      // 将来 mfm-js に増えるノードは、原文のまま出す。
      return toString(node);
  }
}

/**
 * メッセージ本文を、埋め込みカードに出す短い素の文字列にする。
 * 装飾・リンク・コードの記法を外し、改行や連続する空白を 1 つにまとめ、長いときは `maxLength` 文字
 * （コードポイント単位）で切って `…` を付ける。
 */
export function messageExcerpt(content: string, maxLength = EXCERPT_MAX_LENGTH): string {
  let text: string;
  try {
    text = plainText(parse(content));
  } catch {
    // 解析できない本文は、原文をそのまま使う。
    text = content;
  }
  const collapsed = text.replace(/\s+/g, " ").trim();
  const characters = Array.from(collapsed);
  return characters.length > maxLength
    ? `${characters.slice(0, maxLength).join("").trimEnd()}…`
    : collapsed;
}
