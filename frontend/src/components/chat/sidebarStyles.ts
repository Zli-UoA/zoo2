import { cn } from "cn";

export function sidebarItemClassName(active: boolean, unread: boolean): string {
  return cn(
    "group flex h-9 md:h-8 min-w-0 items-center gap-2 rounded-md px-2 text-sm transition-colors outline-none focus-visible:ring-3 focus-visible:ring-ring/50",
    unread && "font-semibold text-sidebar-foreground",
    active
      ? "bg-brand-muted font-semibold text-brand-muted-foreground"
      : unread
        ? "hover:bg-sidebar-accent"
        : "text-muted-foreground hover:bg-sidebar-accent hover:text-sidebar-accent-foreground",
  );
}
