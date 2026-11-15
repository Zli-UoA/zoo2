import { useRouterState } from "@tanstack/react-router";
import type { ReactNode } from "react";
import { AuthGate } from "./AuthGate";
import { PreviewUserSwitcher } from "./PreviewUserSwitcher";

export function RouteAccess({ children }: { children: ReactNode }) {
  const path = useRouterState({ select: (state) => state.location.pathname });
  return /^\/public\/users\/[^/]+\/?$/.test(path) ? (
    <>{children}</>
  ) : (
    <AuthGate>
      {children}
      <PreviewUserSwitcher />
    </AuthGate>
  );
}
