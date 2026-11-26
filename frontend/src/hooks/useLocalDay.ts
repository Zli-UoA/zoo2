import { useSyncExternalStore } from "react";

// All mounted timestamps share one timeout, including when a suspended tab wakes up.
const listeners = new Set<() => void>();
let timer: number;

function readCalendar() {
  return {
    midnight: new Date().setHours(0, 0, 0, 0),
    zone: Intl.DateTimeFormat().resolvedOptions().timeZone,
  };
}

let calendar = readCalendar();

function updateCalendar() {
  const next = readCalendar();
  if (next.midnight !== calendar.midnight || next.zone !== calendar.zone) calendar = next;
}

function snapshot() {
  if (!listeners.size) updateCalendar();
  return calendar;
}

function scheduleMidnight() {
  window.clearTimeout(timer);
  const now = new Date();
  const midnight = new Date(now);
  midnight.setHours(24, 0, 0, 0);
  timer = window.setTimeout(refresh, midnight.getTime() - now.getTime());
}

function refresh() {
  updateCalendar();
  scheduleMidnight();
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  if (listeners.size === 1) {
    updateCalendar();
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
  return new Date(useSyncExternalStore(subscribe, snapshot, snapshot).midnight);
}
