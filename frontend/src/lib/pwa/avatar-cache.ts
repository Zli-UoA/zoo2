const AVATAR_ORIGIN = "https://avater.zliusercontent.com";
const listeners = new Map<string, Set<() => void>>();
const revisions = new Map<string, string>();
let stopListening: (() => void) | undefined;

function avatarURL(src: string | undefined): string | null {
  try {
    const url = new URL(src ?? "");
    if (url.origin !== AVATAR_ORIGIN || url.username || url.password) return null;
    url.searchParams.delete("__zoo_avatar");
    url.hash = "";
    return url.href;
  } catch {
    return null;
  }
}

/** A revision changes the browser's decoded image key while the SW keeps one cache entry. */
export function avatarSource(src: string | undefined): string | undefined {
  const base = avatarURL(src);
  const revision = base ? revisions.get(base) : undefined;
  if (!base || !revision) return src;
  const url = new URL(base);
  url.searchParams.set("__zoo_avatar", revision);
  return url.href;
}

function startListening(): (() => void) | undefined {
  if (typeof window === "undefined" || !("serviceWorker" in navigator)) return undefined;
  const workers = navigator.serviceWorker;
  const update = (event: MessageEvent) => {
    if (event.source !== workers.controller) return;
    const data: unknown = event.data;
    if (!data || typeof data !== "object") return;
    const { type, url, revision } = data as Record<string, unknown>;
    if (
      type !== "AVATAR_UPDATED" ||
      typeof url !== "string" ||
      typeof revision !== "string" ||
      !/^[a-f0-9]{64}$/.test(revision)
    )
      return;
    const base = avatarURL(url);
    if (!base || revisions.get(base) === revision) return;
    revisions.set(base, revision);
    for (const notify of listeners.get(base) ?? []) notify();
    // Retain active images and bound inactive revisions on long-lived sessions.
    if (revisions.size > 256)
      for (const key of revisions.keys()) {
        if (!listeners.has(key)) revisions.delete(key);
        if (revisions.size <= 256) break;
      }
  };
  const refresh = () => {
    if (!workers.controller || document.visibilityState === "hidden" || !navigator.onLine) return;
    const urls = new Set<string>();
    for (const image of document.querySelectorAll<HTMLImageElement>(
      'img[data-slot="avatar-image"]',
    )) {
      if (!image.complete || image.naturalWidth === 0) continue;
      const rect = image.getBoundingClientRect();
      if (
        rect.bottom <= 0 ||
        rect.top >= window.innerHeight ||
        rect.right <= 0 ||
        rect.left >= window.innerWidth
      )
        continue;
      const url = avatarURL(image.src);
      if (url && listeners.has(url)) urls.add(url);
    }
    const values = [...urls];
    for (let offset = 0; offset < values.length; offset += 64)
      workers.controller.postMessage({
        type: "REFRESH_AVATARS",
        urls: values.slice(offset, offset + 64),
      });
  };
  workers.addEventListener("message", update);
  workers.addEventListener("controllerchange", refresh);
  window.addEventListener("focus", refresh);
  window.addEventListener("online", refresh);
  document.addEventListener("visibilitychange", refresh);
  const timer = window.setInterval(refresh, 5 * 60 * 1000);
  return () => {
    workers.removeEventListener("message", update);
    workers.removeEventListener("controllerchange", refresh);
    window.removeEventListener("focus", refresh);
    window.removeEventListener("online", refresh);
    document.removeEventListener("visibilitychange", refresh);
    window.clearInterval(timer);
  };
}

export function subscribeAvatar(src: string | undefined, notify: () => void): () => void {
  const base = avatarURL(src);
  if (!base) return () => {};
  const subscribers = listeners.get(base) ?? new Set<() => void>();
  subscribers.add(notify);
  listeners.set(base, subscribers);
  stopListening ??= startListening();
  return () => {
    subscribers.delete(notify);
    if (subscribers.size === 0) listeners.delete(base);
    if (listeners.size === 0) {
      stopListening?.();
      stopListening = undefined;
    }
  };
}
