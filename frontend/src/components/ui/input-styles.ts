/**
 * Phase 4 は shadcn テーマのみで画面を組むため、入力欄は共通クラスを当てるだけにする。
 * 細かい Tailwind の作り込みは Phase 6 以降。
 */
export function inputClassName(): string {
  return "h-8 rounded-lg border border-border bg-background px-2 text-sm outline-none focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50";
}
