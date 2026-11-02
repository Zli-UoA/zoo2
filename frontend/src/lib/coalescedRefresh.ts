/** Immediate first load, then at most one trailing refresh per interval. */
export function coalescedRefresh(refresh: () => void, interval = 5000) {
  let last = Number.NEGATIVE_INFINITY;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let disposed = false;
  const run = () => {
    timer = undefined;
    if (disposed) return;
    last = Date.now();
    refresh();
  };
  return {
    request: () => {
      if (disposed || timer !== undefined) return;
      const remaining = interval - (Date.now() - last);
      if (remaining <= 0) run();
      else timer = setTimeout(run, remaining);
    },
    dispose: () => {
      disposed = true;
      clearTimeout(timer);
    },
  };
}
