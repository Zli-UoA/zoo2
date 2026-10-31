import { readStorageValue, writeStorageValue } from "./storage";

export type FrontendImplementation = "react" | "compose";
export const FRONTEND_STORAGE_KEY = "zoo-frontend";

/** Disabled builds always retain React, even if a preference survives another deployment. */
export function supportsComposePath(path: string): boolean {
  return path.startsWith("/") && !path.startsWith("//");
}

export function resolveFrontend(
  enabled: boolean,
  search: string,
  stored: string | null,
  pathname = "/",
): FrontendImplementation {
  if (!enabled || !supportsComposePath(pathname)) return "react";
  const requested = new URLSearchParams(search).get("frontend");
  if (requested === "react" || requested === "compose") return requested;
  return stored === "compose" ? "compose" : "react";
}

export function composeEnabled(): boolean {
  return import.meta.env.VITE_EXPERIMENTAL_COMPOSE === "true";
}

export function selectedFrontend(): FrontendImplementation {
  return resolveFrontend(
    composeEnabled(),
    window.location.search,
    readStorageValue(FRONTEND_STORAGE_KEY),
    window.location.pathname,
  );
}

/** Full navigation also releases the canvas runtime, subscriptions and in-memory tokens. */
export function switchFrontend(next: FrontendImplementation): void {
  writeStorageValue(FRONTEND_STORAGE_KEY, next);
  const url = new URL(window.location.href);
  url.searchParams.set("frontend", next);
  window.location.assign(url.href);
}
