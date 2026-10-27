import { MFM_FUNCTIONS } from "./mfmFunctions";

export interface MfmSuggestionRange {
  start: number;
  end: number;
  query: string;
}

/** Suggest at the caret after $, $[, or a partial function name; leave escapes and code alone. */
export function mfmQuery(value: string, caret: number): MfmSuggestionRange | null {
  if (caret < 0 || caret > value.length) return null;
  const prefix = value.slice(0, caret);
  const match = /(?:\$|＄)(?:\[|［)?([\p{L}\p{M}\p{N}]*)$/u.exec(prefix);
  if (!match || /https?:\/\/\S*$/i.test(prefix)) return null;
  let escapes = 0;
  for (let i = match.index - 1; i >= 0 && value[i] === "\\"; i--) escapes++;
  if (escapes % 2) return null;
  let delimiter = 0;
  for (const ticks of prefix.slice(0, match.index).matchAll(/(?<!\\)`+/g)) {
    const length = ticks[0].length;
    if (!delimiter) delimiter = length;
    else if (length === delimiter || (delimiter >= 3 && length >= delimiter)) delimiter = 0;
  }
  return delimiter
    ? null
    : { start: match.index, end: caret, query: match[1]!.normalize("NFKC").toLowerCase() };
}

export function mfmOptions(
  query: string,
  labelFor?: (name: (typeof MFM_FUNCTIONS)[number]) => string,
) {
  const search = query.normalize("NFKC").toLowerCase();
  return MFM_FUNCTIONS.filter(
    (name) =>
      name.startsWith(search) ||
      (search.length > 0 && labelFor?.(name).normalize("NFKC").toLowerCase().includes(search)),
  );
}

const templates: Partial<Record<(typeof MFM_FUNCTIONS)[number], string>> = {
  font: "font.serif",
  scale: "scale.x=2,y=2",
  position: "position.x=1,y=0",
  fg: "fg.color=ff0000",
  bg: "bg.color=ffff00",
  border: "border.width=1,style=solid",
  rotate: "rotate.deg=90",
  clickable: "clickable.ev=click",
};

export function insertMfm(
  value: string,
  range: MfmSuggestionRange,
  name: (typeof MFM_FUNCTIONS)[number],
) {
  const opening = `$[${templates[name] ?? name} `;
  return {
    value: value.slice(0, range.start) + opening + "]" + value.slice(range.end),
    caret: range.start + opening.length,
  };
}
