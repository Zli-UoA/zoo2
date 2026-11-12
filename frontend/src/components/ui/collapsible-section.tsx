import { ChevronRightIcon } from "lucide-react";
import { useId, useState, type ReactNode } from "react";
import { cn } from "cn";

export function CollapsibleSection({
  title,
  icon,
  count,
  actions,
  children,
  className,
  defaultOpen = true,
}: {
  title: string;
  icon: ReactNode;
  count?: ReactNode;
  actions?: ReactNode;
  children: ReactNode;
  className?: string;
  defaultOpen?: boolean;
}) {
  const [open, setOpen] = useState(defaultOpen);
  const headingId = useId();
  const contentId = useId();
  return (
    <section className={className} aria-labelledby={headingId}>
      <div className="flex min-h-9 items-center gap-1 px-2">
        <h2 id={headingId} className="min-w-0 flex-1">
          <button
            type="button"
            aria-expanded={open}
            aria-controls={contentId}
            onClick={() => setOpen((value) => !value)}
            className="flex min-h-9 w-full min-w-0 items-center gap-1.5 rounded-md px-1 text-left text-xs font-semibold text-muted-foreground outline-none hover:bg-sidebar-accent hover:text-sidebar-accent-foreground focus-visible:ring-2 focus-visible:ring-ring/50"
          >
            <ChevronRightIcon
              aria-hidden="true"
              className={cn("size-3 shrink-0 transition-transform", open && "rotate-90")}
            />
            <span
              aria-hidden="true"
              className="flex size-4 shrink-0 items-center justify-center [&>svg]:size-4"
            >
              {icon}
            </span>
            <span className="truncate">{title}</span>
            {count !== undefined ? (
              <span aria-hidden="true" className="ml-auto shrink-0 px-1 font-normal tabular-nums">
                {count}
              </span>
            ) : null}
          </button>
        </h2>
        {actions ? <div className="flex shrink-0 items-center gap-0.5">{actions}</div> : null}
      </div>
      <div id={contentId} hidden={!open}>
        {children}
      </div>
    </section>
  );
}
