import { Outlet, createFileRoute, useRouterState } from "@tanstack/react-router";
import { useEffect, useState } from "react";

import { AppShell } from "../components/layout/AppShell";
import { OfflineProvider } from "../components/offline/OfflineProvider";
import { ShellSkeleton } from "../components/layout/ShellSkeleton";
import { resolveConfig } from "../lib/config";
import {
  selectedFrontend,
  composeEnabled,
  FRONTEND_STORAGE_KEY,
} from "../lib/experimentalFrontend";
import { writeStorageValue } from "../lib/storage";
import { ComposeFrontend } from "../components/experimental/ComposeFrontend";

export const Route = createFileRoute("/_shell")({ component: ShellLayout });

/**
 * シェル配下のレイアウト。Electric の購読はブラウザ専用なので、マウント後に
 * 一度だけ `AppShell` を描画する（SSR はスケルトン）。
 */
function ShellLayout() {
  const path = useRouterState({ select: (state) => state.location.pathname });
  const [mounted, setMounted] = useState(false);
  const [currentImplementation, setImplementation] = useState<"react" | "compose">("react");
  // AuthGate が設定を検証済み。ここでは同一インスタンスを使い回し、
  // OfflineProvider の初期化が再レンダーで繰り返されないようにする。
  const [config] = useState(resolveConfig);
  useEffect(() => {
    const implementation = selectedFrontend();
    const requested = new URLSearchParams(window.location.search).get("frontend");
    if (composeEnabled() && (requested === "react" || requested === "compose"))
      writeStorageValue(FRONTEND_STORAGE_KEY, requested);
    if (mounted && implementation !== currentImplementation) {
      // Cross-runtime navigation must release the old Electric collections and
      // Canvas/input owners, just like the explicit frontend switch.
      window.location.reload();
      return;
    }
    setImplementation(implementation);
    setMounted(true);
  }, [path, mounted, currentImplementation]);

  return mounted && currentImplementation === "compose" ? (
    <ComposeFrontend />
  ) : mounted ? (
    <OfflineProvider config={config}>
      <AppShell>
        <Outlet />
      </AppShell>
    </OfflineProvider>
  ) : (
    <ShellSkeleton />
  );
}
