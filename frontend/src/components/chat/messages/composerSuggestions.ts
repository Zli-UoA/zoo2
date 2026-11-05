import { isInsideCode, type SuggestionRange } from "./suggestionContext";

export interface CommandSuggestion {
  app_id: string;
  app_name: string;
  name: string;
}

/** Boundaries keep URL fragments, headings and paths out of channel/command completion. */
export function composerQuery(
  value: string,
  caret: number,
  kind: "channel" | "command",
): SuggestionRange | null {
  if (caret < 0 || caret > value.length) return null;
  const prefix = value.slice(0, caret);
  const match =
    kind === "command"
      ? /^\/([a-zA-Z0-9_-]*)$/.exec(prefix)
      : /(?:^|[\s([{（「『])#([^\s#.,!?<>()[\]{}「」『』、。！？]*)$/u.exec(prefix);
  if (!match) return null;
  const start = caret - match[1]!.length - 1;
  return isInsideCode(value, start)
    ? null
    : { start, end: caret, query: match[1]!.normalize("NFKC").toLowerCase() };
}

export function insertSuggestion(value: string, range: SuggestionRange, token: string) {
  // Replace the rest of a partially edited token as well as the prefix before the caret.
  const suffix = /^[^\s.,!?<>()[\]{}「」『』、。！？]*/u.exec(value.slice(range.end))![0];
  const after = value.slice(range.end + suffix.length);
  const spacing = after.startsWith(" ") ? "" : " ";
  return {
    value: value.slice(0, range.start) + token + spacing + after,
    caret: range.start + token.length + 1,
  };
}
