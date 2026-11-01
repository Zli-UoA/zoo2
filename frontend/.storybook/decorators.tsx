import type { Decorator } from "@storybook/react-vite";
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  RouterProvider,
} from "@tanstack/react-router";
import { setupWorker } from "msw/browser";
import type { RequestHandler, WebSocketHandler } from "msw";
import { createContext, useContext, useMemo, useState, type ReactNode } from "react";

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

const StoryContent = createContext<ReactNode>(null);
function StoryRouteContent() {
  return <>{useContext(StoryContent)}</>;
}

export function StoryRouter({
  children,
  initialEntries = ["/"],
}: {
  children: ReactNode;
  initialEntries?: string[];
}) {
  const entriesKey = initialEntries.join("\n");
  const router = useMemo(() => {
    const rootRoute = createRootRoute({ component: StoryRouteContent });
    const channelRoute = createRoute({
      getParentRoute: () => rootRoute,
      path: "channels/$channelId",
    });
    const settingsRoute = createRoute({
      getParentRoute: () => rootRoute,
      path: "settings",
      validateSearch: (search: Record<string, unknown>) => search,
    });
    const homeRoute = createRoute({ getParentRoute: () => rootRoute, path: "home" });
    const notificationsRoute = createRoute({
      getParentRoute: () => rootRoute,
      path: "notifications",
    });
    const appRoute = createRoute({ getParentRoute: () => rootRoute, path: "apps/$appId" });
    const consentRoute = createRoute({ getParentRoute: () => rootRoute, path: "apps-authorize" });
    return createRouter({
      routeTree: rootRoute.addChildren([
        homeRoute,
        createRoute({ getParentRoute: () => rootRoute, path: "home/posts/$postId" }),
        createRoute({ getParentRoute: () => rootRoute, path: "home/users/$actorId" }),
        channelRoute,
        settingsRoute,
        notificationsRoute,
        appRoute,
        consentRoute,
      ]),
      history: createMemoryHistory({ initialEntries: entriesKey.split("\n") }),
    });
  }, [entriesKey]);
  return (
    <StoryContent.Provider value={children}>
      <RouterProvider router={router} />
    </StoryContent.Provider>
  );
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

let workerStart: Promise<void> | null = null;

/** 最初の Story がレンダーされる前に MSW を有効化する。 */
export function startStorybookWorker(): Promise<void> {
  workerStart ??= worker
    .start({ onUnhandledFrame: "bypass", quiet: true })
    .then(() => undefined)
    .catch((error) => {
      workerStart = null;
      throw error;
    });
  return workerStart;
}

export const withMsw: Decorator = (Story, context) => {
  const handlers = (context.parameters.msw?.handlers ?? []) as (
    | RequestHandler
    | WebSocketHandler
  )[];
  // MSW は先に一致したハンドラーを使うため、Story 固有のハンドラーを先に置く。
  worker.resetHandlers(...handlers, ...storybookApiHandlers);
  return <Story />;
};
