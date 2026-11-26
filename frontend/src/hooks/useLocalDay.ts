import { useSyncExternalStore } from "react";

// All mounted timestamps share one timeout, including when a suspended tab wakes up.
const listeners = new Set<() => void>();
let timer: number;

function startOfToday() {
  return new Date().setHours(0, 0, 0, 0);
}

function scheduleMidnight() {
  window.clearTimeout(timer);
  const now = new Date();
  const midnight = new Date(now);
  midnight.setHours(24, 0, 0, 0);
  timer = window.setTimeout(refresh, midnight.getTime() - now.getTime());
}

function refresh() {
  scheduleMidnight();
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  if (listeners.size === 1) {
    scheduleMidnight();
    document.addEventListener("visibilitychange", refresh);
    window.addEventListener("focus", refresh);
  }
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0) {
      window.clearTimeout(timer);
      document.removeEventListener("visibilitychange", refresh);
      window.removeEventListener("focus", refresh);
    }
  };
}

export function useLocalDay() {
  return new Date(useSyncExternalStore(subscribe, startOfToday, startOfToday));
}
