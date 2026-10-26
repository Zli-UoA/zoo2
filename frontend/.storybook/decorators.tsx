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
import { CollectionsProvider } from "../src/lib/electric/collections-context";
import { LOCALE_STORAGE_KEY, type Locale } from "../src/lib/i18n/locale";
import { createStoryCollections, storyCollections, type StoryCollectionsInput } from "./fixtures";
import { storybookApiHandlers } from "./mock-api";

export const worker = setupWorker();

function Collections({
  children,
  fixture,
}: {
  children: ReactNode;
  fixture: StoryCollectionsInput;
}) {
  const bundle = useMemo(() => createStoryCollections(fixture), [fixture]);
  return <CollectionsProvider value={bundle}>{children}</CollectionsProvider>;
}

/** 画面系 story に fixture のローカルコレクションを注入する。 */
export const withCollections: Decorator = (Story, context) => {
  const fixture = (context.parameters.collections ?? storyCollections) as StoryCollectionsInput;
  return (
    <Collections fixture={fixture}>
      <Story />
    </Collections>
  );
};

export function StoryRouter({
  children,
  initialEntries = ["/"],
}: {
  children: ReactNode;
  initialEntries?: string[];
}) {
  const router = useMemo(() => {
    const rootRoute = createRootRoute({ component: () => <>{children}</> });
    return createRouter({
      routeTree: rootRoute,
      history: createMemoryHistory({ initialEntries }),
    });
  }, [children, initialEntries]);
  return <RouterProvider router={router} />;
}

function Providers({
  children,
  theme,
  locale,
  initialEntries,
}: {
  children: ReactNode;
  theme: Theme;
  locale: Locale;
  initialEntries: string[];
}) {
  useState(() => {
    localStorage.setItem(THEME_STORAGE_KEY, theme);
    localStorage.setItem(LOCALE_STORAGE_KEY, locale);
    return null;
  });
  return (
    <ThemeProvider>
      <LocaleProvider>
        <StoryRouter initialEntries={initialEntries}>{children}</StoryRouter>
      </LocaleProvider>
    </ThemeProvider>
  );
}

export const withProviders: Decorator = (Story, context) => {
  const theme = (context.globals.theme ?? "light") as Theme;
  const locale = (context.globals.locale ?? "ja") as Locale;
  const initialEntries = (context.parameters.router?.initialEntries ?? ["/"]) as string[];
  return (
    <Providers
      theme={theme}
      locale={locale}
      initialEntries={initialEntries}
      key={`${theme}-${locale}-${initialEntries.join(",")}`}
    >
      <Story />
    </Providers>
  );
};

let workerStarted = false;

export const withMsw: Decorator = (Story, context) => {
  const handlers = (context.parameters.msw?.handlers ?? []) as RequestHandler[];
  worker.resetHandlers(...storybookApiHandlers, ...handlers);
  if (!workerStarted) {
    workerStarted = true;
    void worker.start({ onUnhandledFrame: "bypass", quiet: true });
  }
  return <Story />;
};
