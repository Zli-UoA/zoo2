import { Fragment, type ReactNode } from "react";

import { highlightSegments } from "../../lib/searchHighlight";

/** 検索語に当たった部分を `<mark>` で強調する。語がなければ文字列をそのまま返す。 */
export function highlightText(text: string, terms: readonly string[] | undefined): ReactNode {
  if (!terms?.length) return text;
  const segments = highlightSegments(text, terms);
  if (segments.length === 1 && !segments[0]!.match) return text;
  return segments.map((segment, index) =>
    segment.match ? (
      <mark
        key={index}
        className="rounded-sm bg-highlight px-px text-highlight-foreground"
        data-search-highlight=""
      >
        {segment.text}
      </mark>
    ) : (
      <Fragment key={index}>{segment.text}</Fragment>
    ),
  );
}
