import { Outlet, createFileRoute } from "@tanstack/react-router";
import { useEffect, useState } from "react";

import { AppShell } from "../components/layout/AppShell";
import { ShellSkeleton } from "../components/layout/ShellSkeleton";

export const Route = createFileRoute("/_shell")({ component: ShellLayout });

/**
 * シェル配下のレイアウト。Electric の購読はブラウザ専用なので、マウント後に
 * 一度だけ `AppShell` を描画する（SSR はスケルトン）。
 */
function ShellLayout() {
  const [mounted, setMounted] = useState(false);
  useEffect(() => {
    setMounted(true);
  }, []);

  return mounted ? (
    <AppShell>
      <Outlet />
    </AppShell>
  ) : (
    <ShellSkeleton />
  );
}
