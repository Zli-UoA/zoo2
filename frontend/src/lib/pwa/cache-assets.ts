/** Capture late fonts/Wasm without resending the full list on API/Electric polling. */
export function startAssetCaching(): () => void {
  if (typeof navigator === "undefined" || !("serviceWorker" in navigator)) return () => {};
  let stopped = false;
  let controller: ServiceWorker | null = null;
  const known = new Set<string>();
  const sent = new Set<string>();
  let timer: ReturnType<typeof setTimeout> | undefined;
  const collect = (entries: readonly PerformanceEntry[]) => {
    const size = known.size;
    for (const entry of entries) {
      try {
        const url = new URL(entry.name);
        if (
          url.origin === window.location.origin &&
          (url.pathname.startsWith("/assets/") || url.pathname.startsWith("/compose/"))
        )
          known.add(url.pathname);
      } catch {
        /* Ignore malformed resource URLs. */
      }
    }
    return known.size !== size;
  };
  const send = () => {
    if (stopped || !navigator.serviceWorker.controller) return;
    if (controller !== navigator.serviceWorker.controller) {
      controller = navigator.serviceWorker.controller;
      sent.clear();
    }
    collect(performance.getEntriesByType("resource"));
    const pending = [...known].filter((url) => !sent.has(url));
    for (let offset = 0; offset < pending.length; offset += 200) {
      const urls = pending.slice(offset, offset + 200);
      controller.postMessage({ type: "CACHE_ASSETS", urls });
      urls.forEach((url) => sent.add(url));
    }
  };
  const observer =
    typeof PerformanceObserver === "undefined"
      ? null
      : new PerformanceObserver((entries) => {
          if (!collect(entries.getEntries())) return;
          clearTimeout(timer);
          timer = setTimeout(send, 300);
        });
  observer?.observe({ type: "resource", buffered: true });
  const timers = [setTimeout(send, 1_000), setTimeout(send, 4_000)];
  navigator.serviceWorker.addEventListener("controllerchange", send);
  window.addEventListener("load", send);
  void navigator.serviceWorker.ready.then(send).catch(() => {});
  return () => {
    stopped = true;
    observer?.disconnect();
    clearTimeout(timer);
    timers.forEach(clearTimeout);
    navigator.serviceWorker.removeEventListener("controllerchange", send);
    window.removeEventListener("load", send);
  };
}
