import type { ReactNode } from "react";
import { cn } from "cn";

export function SettingsSection({
  title,
  description,
  children,
  className,
}: {
  title?: string;
  description?: string;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section className={cn("space-y-3 rounded-lg border p-4", className)}>
      {title ? (
        <div>
          <h3 className="text-sm font-semibold">{title}</h3>
          {description ? <p className="mt-1 text-xs text-muted-foreground">{description}</p> : null}
        </div>
      ) : null}
      {children}
    </section>
  );
}

export function SettingRow({
  icon,
  label,
  description,
  children,
}: {
  icon?: ReactNode;
  label: string;
  description?: string;
  children: ReactNode;
}) {
  return (
    <div className="flex min-h-16 items-start justify-between gap-4 p-4">
      <div className="flex min-h-8 min-w-0 items-center gap-3">
        {icon ? (
          <span aria-hidden="true" className="shrink-0 text-muted-foreground [&>svg]:size-5">
            {icon}
          </span>
        ) : null}
        <div className="min-w-0">
          <p className="text-sm font-medium">{label}</p>
          {description ? (
            <p className="mt-1 break-words text-xs text-muted-foreground">{description}</p>
          ) : null}
        </div>
      </div>
      <div className="flex shrink-0 items-center gap-2">{children}</div>
    </div>
  );
}

export const panelTabClassName =
  "rounded-md px-3 py-2 text-sm font-medium text-muted-foreground outline-none transition-colors hover:text-foreground focus-visible:ring-3 focus-visible:ring-ring/50 data-[active]:bg-background data-[active]:text-foreground data-[active]:shadow-sm";
