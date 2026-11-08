import { readStorageValue, writeStorageValue } from "./storage";

export type FrontendImplementation = "react" | "compose";
export const FRONTEND_STORAGE_KEY = "zoo-frontend";

export function supportsComposePath(path: string): boolean {
  return (
    import.meta.env.VITE_EXPERIMENTAL_COMPOSE === "true" &&
    path.startsWith("/") &&
    !path.startsWith("//")
  );
}

export function resolveFrontend(
  search: string,
  stored: string | null,
  pathname = "/",
): FrontendImplementation {
  if (!supportsComposePath(pathname)) return "react";
  const requested = new URLSearchParams(search).get("frontend");
  if (requested === "react" || requested === "compose") return requested;
  return stored === "compose" ? "compose" : "react";
}

export function selectedFrontend(): FrontendImplementation {
  return resolveFrontend(
    window.location.search,
    readStorageValue(FRONTEND_STORAGE_KEY),
    window.location.pathname,
  );
}

/** Full navigation also releases the canvas runtime, subscriptions and in-memory tokens. */
export function switchFrontend(next: FrontendImplementation): void {
  if (import.meta.env.VITE_EXPERIMENTAL_COMPOSE !== "true") next = "react";
  writeStorageValue(FRONTEND_STORAGE_KEY, next);
  const url = new URL(window.location.href);
  url.searchParams.set("frontend", next);
  window.location.assign(url.href);
}
