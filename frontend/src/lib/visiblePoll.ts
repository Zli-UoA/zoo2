/** Poll only while visible, serializing requests and backing off when nothing is active. */
export function visiblePoll(
  load: (signal: AbortSignal) => Promise<boolean | void>,
  { interval, maxInterval = interval }: { interval: number; maxInterval?: number },
) {
  let stopped = false;
  let running = false;
  let pending = false;
  let delay = interval;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let controller: AbortController | undefined;
  const visible = () => document.visibilityState !== "hidden";
  const run = async () => {
    clearTimeout(timer);
    if (stopped || !visible()) return;
    if (running) {
      pending = true;
      return;
    }
    running = true;
    pending = false;
    const request = new AbortController();
    controller = request;
    try {
      const active = await load(AbortSignal.any([request.signal, AbortSignal.timeout(10000)]));
      delay = active ? interval : Math.min(maxInterval, delay * 2);
    } catch {
      delay = Math.min(maxInterval, delay * 2);
    } finally {
      running = false;
      if (!stopped && visible()) {
        if (pending) {
          delay = interval;
          void run();
        } else timer = setTimeout(() => void run(), delay);
      }
    }
  };
  const refresh = () => {
    delay = interval;
    void run();
  };
  const onVisibility = () => {
    if (visible()) refresh();
    else {
      clearTimeout(timer);
      pending = false;
      controller?.abort();
    }
  };
  document.addEventListener("visibilitychange", onVisibility);
  window.addEventListener("online", refresh);
  refresh();
  return {
    refresh,
    dispose: () => {
      stopped = true;
      pending = false;
      clearTimeout(timer);
      controller?.abort();
      document.removeEventListener("visibilitychange", onVisibility);
      window.removeEventListener("online", refresh);
    },
  };
}
