import type { ComponentProps } from "react";
import { cn } from "cn";

export const inlineLinkClassName =
  "rounded-sm text-link underline-offset-2 outline-none focus-visible:ring-2 focus-visible:ring-ring/50";

/** Callers validate destinations; this component shares external-link behavior and styling. */
export function ExternalLink({ className, ...props }: Omit<ComponentProps<"a">, "target" | "rel">) {
  return (
    <a
      {...props}
      target="_blank"
      rel="noopener noreferrer"
      className={cn(inlineLinkClassName, className)}
    />
  );
}
