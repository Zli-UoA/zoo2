import type { Decorator } from "@storybook/react-vite";
import {
  createMemoryHistory,
  createRootRoute,
  createRouter,
  RouterProvider,
} from "@tanstack/react-router";
import { setupWorker } from "msw/browser";
import type { RequestHandler } from "msw";
import { useMemo, useState, type ReactNode } from "react";

import { LocaleProvider } from "../src/components/i18n/LocaleProvider";
import { ThemeProvider } from "../src/components/theme/ThemeProvider";
import { THEME_STORAGE_KEY, type Theme } from "../src/components/theme/theme";
import { LOCALE_STORAGE_KEY, type Locale } from "../src/lib/i18n/locale";

export const worker = setupWorker();

function StoryRouter({ children }: { children: ReactNode }) {
  const router = useMemo(() => {
    const rootRoute = createRootRoute({ component: () => <>{children}</> });
    return createRouter({
      routeTree: rootRoute,
      history: createMemoryHistory({ initialEntries: ["/"] }),
    });
  }, [children]);
  return <RouterProvider router={router} />;
}

function Providers({
  children,
  theme,
  locale,
}: {
  children: ReactNode;
  theme: Theme;
  locale: Locale;
}) {
  useState(() => {
    localStorage.setItem(THEME_STORAGE_KEY, theme);
    localStorage.setItem(LOCALE_STORAGE_KEY, locale);
    return null;
  });
  return (
    <ThemeProvider>
      <LocaleProvider>
        <StoryRouter>{children}</StoryRouter>
      </LocaleProvider>
    </ThemeProvider>
  );
}

export const withProviders: Decorator = (Story, context) => {
  const theme = (context.globals.theme ?? "light") as Theme;
  const locale = (context.globals.locale ?? "ja") as Locale;
  return (
    <Providers theme={theme} locale={locale} key={`${theme}-${locale}`}>
      <Story />
    </Providers>
  );
};

let workerStarted = false;

export const withMsw: Decorator = (Story, context) => {
  const handlers = (context.parameters.msw?.handlers ?? []) as RequestHandler[];
  worker.resetHandlers(...handlers);
  if (!workerStarted && !import.meta.env.VITEST) {
    workerStarted = true;
    void worker.start({ onUnhandledFrame: "bypass", quiet: true });
  }
  return <Story />;
};
