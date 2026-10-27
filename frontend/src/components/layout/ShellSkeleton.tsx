import { Skeleton } from "../ui/skeleton";

const SIDEBAR_ROWS = [0, 1, 2, 3, 4, 5];

/** シェルのデータ購読が始まるまでのプレースホルダ。3ペインの形だけをなぞる。 */
export function ShellSkeleton() {
  return (
    <div data-app-shell className="flex h-dvh w-full overflow-hidden bg-background">
      <div className="w-14 shrink-0 border-r bg-sidebar md:w-16" />
      <div className="hidden w-72 shrink-0 flex-col gap-2 border-r p-3 md:flex">
        <Skeleton className="h-6 w-24" />
        {SIDEBAR_ROWS.map((row) => (
          <Skeleton key={row} className="h-9 w-full rounded-lg" />
        ))}
      </div>
      <div className="flex min-w-0 flex-1 flex-col">
        <div className="flex items-center gap-3 border-b px-4 py-3">
          <Skeleton className="size-8 rounded-lg" />
          <div className="flex flex-1 flex-col gap-1.5">
            <Skeleton className="h-4 w-40" />
            <Skeleton className="h-3 w-24" />
          </div>
        </div>
        <div className="flex flex-1 flex-col gap-3 p-4">
          <Skeleton className="h-16 w-full rounded-lg" />
          <Skeleton className="h-16 w-3/4 rounded-lg" />
          <Skeleton className="h-16 w-1/2 rounded-lg" />
        </div>
      </div>
    </div>
  );
}
