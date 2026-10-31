import type { ReactNode } from "react";

export const composerTextareaClassName =
  "min-w-0 flex-1 resize-none rounded-none border-0 bg-transparent text-base leading-6 shadow-none field-sizing-fixed focus-visible:ring-0 md:text-sm dark:bg-transparent";

/** Shared input frame; each composer owns its draft, action and keyboard behavior. */
export function ComposerField({ children }: { children: ReactNode }) {
  return (
    <div className="flex items-end gap-2 rounded-xl border border-input bg-background p-1.5 transition-colors focus-within:border-ring focus-within:ring-3 focus-within:ring-ring/30 dark:bg-input/30">
      {children}
    </div>
  );
}
