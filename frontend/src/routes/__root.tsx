import { useEffect } from "react";
import {
  ClientOnly,
  HeadContent,
  Scripts,
  createRootRoute,
  useRouterState,
} from "@tanstack/react-router";
import { TanStackRouterDevtoolsPanel } from "@tanstack/react-router-devtools";
import { TanStackDevtools } from "@tanstack/react-devtools";

import { AuthGate } from "@/components/auth/AuthGate";
import { PreviewUserSwitcher } from "@/components/auth/PreviewUserSwitcher";
import { ThemeProvider } from "@/components/theme/ThemeProvider";
import { themeInitScript } from "@/components/theme/theme";
import { Toaster } from "@/components/ui/sonner";
import { TooltipProvider } from "@/components/ui/tooltip";

import { ShellSkeleton } from "../components/layout/ShellSkeleton";
import { resolvePublicConfig } from "../lib/config";
import { localeInitScript } from "../lib/i18n/locale";
import { LocaleProvider } from "../components/i18n/LocaleProvider";
import { installBrowserTelemetry } from "../lib/telemetry/capture";
import { TelemetryErrorBoundary } from "../lib/telemetry/errorBoundary";

import appCss from "../styles.css?url";

export const Route = createRootRoute({
  head: () => ({
    meta: [
      { name: "theme-color", content: "#FFD600" },
      { name: "apple-mobile-web-app-title", content: "Zoo" },
      {
        charSet: "utf-8",
      },
      {
        name: "viewport",
        content: "width=device-width, initial-scale=1",
      },
      {
        title: "Zoo",
      },
    ],
    links: [
      { rel: "shortcut icon", href: "/favicon.ico" },
      { rel: "icon", type: "image/png", sizes: "32x32", href: "/icons/favicon-32.png" },
      { rel: "icon", type: "image/svg+xml", sizes: "any", href: "/icons/zoo.svg" },
      { rel: "manifest", href: "/manifest.webmanifest" },
      { rel: "apple-touch-icon", href: "/icons/icon-180.png" },
      {
        rel: "stylesheet",
        href: appCss,
      },
    ],
  }),
  shellComponent: RootDocument,
});

function RootDocument({ children }: { children: React.ReactNode }) {
  useEffect(() => {
    return installBrowserTelemetry(resolvePublicConfig());
  }, []);

  return (
    <html lang="ja" suppressHydrationWarning>
      <head>
        <script dangerouslySetInnerHTML={{ __html: localeInitScript }} />
        <HeadContent />
        <script dangerouslySetInnerHTML={{ __html: themeInitScript }} />
      </head>
      <body>
        <ClientOnly fallback={<ShellSkeleton />}>
          <LocaleProvider>
            <ThemeProvider>
              <TooltipProvider>
                <TelemetryErrorBoundary>
                  <RouteAccess>{children}</RouteAccess>
                </TelemetryErrorBoundary>
                <Toaster />
              </TooltipProvider>
            </ThemeProvider>
          </LocaleProvider>
        </ClientOnly>
        <TanStackDevtools
          config={{
            position: "bottom-right",
          }}
          plugins={[
            {
              name: "Tanstack Router",
              render: <TanStackRouterDevtoolsPanel />,
            },
          ]}
        />
        <Scripts />
      </body>
    </html>
  );
}

function RouteAccess({ children }: { children: React.ReactNode }) {
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
