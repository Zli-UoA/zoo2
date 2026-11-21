import type { ReactNode } from "react";

export const composerTextareaClassName =
  "min-w-0 flex-1 resize-none rounded-none border-0 bg-transparent text-base leading-6 shadow-none focus-visible:ring-0 md:text-sm dark:bg-transparent";

/**
 * An empty field must not size itself to a wrapping placeholder. Non-empty drafts keep the native
 * content sizing so that reflowing text (a wider pane, a smaller font) updates the height by itself.
 */
export function composerTextareaSizingClass(value: string) {
  return value.length === 0 ? "field-sizing-fixed" : "field-sizing-content";
}

/** Shared input frame; each composer owns its draft, action and keyboard behavior. */
export function ComposerField({ children }: { children: ReactNode }) {
  return (
    <div className="flex items-start gap-2 rounded-xl border border-input bg-background p-1.5 transition-colors focus-within:border-ring focus-within:ring-3 focus-within:ring-ring/30 dark:bg-input/30">
      {children}
    </div>
  );
}
