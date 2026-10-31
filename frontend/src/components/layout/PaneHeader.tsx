import type { ComponentProps, ReactNode } from "react";
import type { LucideIcon } from "lucide-react";
import { cn } from "cn";

/** The rail's 36px logo, 12px above and below, and 1px divider define the header height. */
export function PaneHeader({ className, ...props }: ComponentProps<"header">) {
  return (
    <header
      className={cn(
        "flex h-[calc(--spacing(15)+1px)] shrink-0 items-center gap-3 border-b px-4",
        className,
      )}
      {...props}
    />
  );
}

export function ScreenHeader({
  icon: Icon,
  iconClassName,
  title,
  titleId,
  description,
  actions,
}: {
  icon: LucideIcon;
  iconClassName?: string;
  title: string;
  titleId: string;
  description?: string;
  actions?: ReactNode;
}) {
  return (
    <PaneHeader className="bg-background/80 backdrop-blur supports-backdrop-filter:bg-background/60">
      <div
        className={cn(
          "flex size-8 shrink-0 items-center justify-center rounded-lg bg-brand-muted text-brand-muted-foreground",
          iconClassName,
        )}
        aria-hidden="true"
      >
        <Icon className="size-4" />
      </div>
      <div className="min-w-0 flex-1">
        <h1 id={titleId} className="truncate text-sm font-semibold">
          {title}
        </h1>
        {description ? (
          <p className="truncate text-xs text-muted-foreground">{description}</p>
        ) : null}
      </div>
      {actions}
    </PaneHeader>
  );
}
