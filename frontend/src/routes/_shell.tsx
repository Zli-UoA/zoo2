import { Outlet, createFileRoute } from "@tanstack/react-router";
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
  const [mounted, setMounted] = useState(false);
  const [implementation, setImplementation] = useState<"react" | "compose">("react");
  // AuthGate が設定を検証済み。ここでは同一インスタンスを使い回し、
  // OfflineProvider の初期化が再レンダーで繰り返されないようにする。
  const [config] = useState(resolveConfig);
  useEffect(() => {
    const implementation = selectedFrontend();
    if (composeEnabled() && new URLSearchParams(window.location.search).has("frontend"))
      writeStorageValue(FRONTEND_STORAGE_KEY, implementation);
    setImplementation(implementation);
    setMounted(true);
  }, []);

  return mounted && implementation === "compose" ? (
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
