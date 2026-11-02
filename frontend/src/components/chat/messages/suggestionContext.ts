export interface SuggestionRange {
  start: number;
  end: number;
  query: string;
}

/** Share code-boundary detection across composer completion kinds. */
export function isInsideCode(value: string, position: number): boolean {
  let delimiter = 0;
  for (const ticks of value.slice(0, position).matchAll(/(?<!\\)`+/g)) {
    const length = ticks[0].length;
    if (!delimiter) delimiter = length;
    else if (length === delimiter || (delimiter >= 3 && length >= delimiter)) delimiter = 0;
  }
  return delimiter !== 0;
}
