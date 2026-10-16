/** 表示名・ユーザー id からアバター用の頭文字を作る。 */
export function initials(value: string): string {
  const trimmed = value.trim();
  if (!trimmed) {
    return "?";
  }
  const letters = trimmed
    .split(/\s+/)
    .slice(0, 2)
    .map((part) => Array.from(part)[0] ?? "")
    .join("");
  return letters.toUpperCase() || "?";
}
