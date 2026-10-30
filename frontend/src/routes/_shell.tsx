import { Outlet, createFileRoute } from "@tanstack/react-router";
import { useEffect, useState } from "react";

import { AppShell } from "../components/layout/AppShell";
import { OfflineProvider } from "../components/offline/OfflineProvider";
import { ShellSkeleton } from "../components/layout/ShellSkeleton";
import { CallProvider } from "../lib/call/call-context";
import { resolveConfig } from "../lib/config";

export const Route = createFileRoute("/_shell")({ component: ShellLayout });

/**
 * シェル配下のレイアウト。Electric の購読はブラウザ専用なので、マウント後に
 * 一度だけ `AppShell` を描画する（SSR はスケルトン）。
 */
function ShellLayout() {
  const [mounted, setMounted] = useState(false);
  // AuthGate が設定を検証済み。ここでは同一インスタンスを使い回し、
  // OfflineProvider の初期化が再レンダーで繰り返されないようにする。
  const [config] = useState(resolveConfig);
  useEffect(() => {
    setMounted(true);
  }, []);

  return mounted ? (
    <OfflineProvider config={config}>
      <CallProvider config={config}>
        <AppShell>
          <Outlet />
        </AppShell>
      </CallProvider>
    </OfflineProvider>
  ) : (
    <ShellSkeleton />
  );
}
