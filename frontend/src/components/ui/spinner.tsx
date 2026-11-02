import { LoaderCircleIcon } from "lucide-react";
import { cn } from "cn";

export function Spinner({ label, className }: { label: string; className?: string }) {
  return (
    <div role="status" className={cn("flex items-center justify-center", className)}>
      <LoaderCircleIcon
        className="size-7 animate-spin text-muted-foreground motion-reduce:animate-none"
        aria-hidden="true"
      />
      <span className="sr-only">{label}</span>
    </div>
  );
}
