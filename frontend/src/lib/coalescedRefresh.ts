/** Immediate first load, then one trailing refresh per interval, without overlapping requests. */
export function coalescedRefresh(refresh: () => void | Promise<void>, interval = 5000) {
  let last = Number.NEGATIVE_INFINITY;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let disposed = false;
  let running = false;
  let pending = false;
  const request = () => {
    if (disposed) return;
    if (running) {
      pending = true;
      return;
    }
    if (timer !== undefined) return;
    const remaining = interval - (Date.now() - last);
    if (remaining <= 0) run();
    else timer = setTimeout(run, remaining);
  };
  const complete = () => {
    running = false;
    if (pending) {
      pending = false;
      request();
    }
  };
  const run = () => {
    timer = undefined;
    if (disposed) return;
    last = Date.now();
    running = true;
    try {
      const result = refresh();
      if (result) void result.then(complete, complete);
      else complete();
    } catch (error) {
      complete();
      throw error;
    }
  };
  return {
    request,
    dispose: () => {
      disposed = true;
      pending = false;
      clearTimeout(timer);
    },
  };
}
