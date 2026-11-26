import { useSyncExternalStore } from "react";

// All mounted timestamps share one timeout, including when a suspended tab wakes up.
const listeners = new Set<() => void>();
let timer: number;
const visibleCheckInterval = 60_000;

function readCalendar() {
  return {
    midnight: new Date().setHours(0, 0, 0, 0),
    zone: Intl.DateTimeFormat().resolvedOptions().timeZone,
  };
}

let calendar = readCalendar();

function updateCalendar() {
  const next = readCalendar();
  if (next.midnight !== calendar.midnight || next.zone !== calendar.zone) {
    calendar = next;
    return true;
  }
  return false;
}

function snapshot() {
  if (!listeners.size) updateCalendar();
  return calendar;
}

function scheduleCheck() {
  window.clearTimeout(timer);
  const now = new Date();
  const midnight = new Date(now);
  midnight.setHours(24, 0, 0, 0);
  const untilMidnight = midnight.getTime() - now.getTime();
  timer = window.setTimeout(
    refresh,
    document.hidden ? untilMidnight : Math.min(untilMidnight, visibleCheckInterval),
  );
}

function refresh() {
  const changed = updateCalendar();
  scheduleCheck();
  if (changed) for (const listener of listeners) listener();
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  if (listeners.size === 1) {
    updateCalendar();
    scheduleCheck();
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

export function useLocalTimeZone() {
  return useSyncExternalStore(subscribe, snapshot, snapshot).zone;
}

export function useLocalDay() {
  return new Date(useSyncExternalStore(subscribe, snapshot, snapshot).midnight);
}
