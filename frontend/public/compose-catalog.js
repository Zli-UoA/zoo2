// Each story owns an iframe: leaving the story destroys its Canvas, input owner and font tasks.
const status = document.getElementById("status");
const send = (state, defaultMessage) =>
  parent.postMessage(
    { type: "zoo-compose-catalog-status", state, defaultMessage },
    location.origin,
  );
const callbackSession = crypto.randomUUID();
let callbackSequence = 0;
const callbacks = new Map();
let update;
let close;
let escape;
let started = false;
const timeout = setTimeout(() => fail("Composeの読み込みがタイムアウトしました。"), 90_000);
function fail(message) {
  const labels = globalThis.zooCatalog.options.labels ?? {};
  clearTimeout(timeout);
  status.textContent = `${labels["experimental.catalogLoadError"] ?? message} ${labels["experimental.catalogBuildHint"] ?? "Install JDK 21 and run pnpm build:compose in frontend."}`;
  send("error");
}
let browserFiles;
let focusedFileTarget = "";
const fileTargets = new Map();
const fileWaiters = [];
const filesReady = () =>
  browserFiles
    ? Promise.resolve(browserFiles)
    : new Promise((resolve, reject) => fileWaiters.push({ resolve, reject }));
globalThis.zooCatalog = {
  options: {},
  installFiles(files) {
    browserFiles = files;
    for (const [id, target] of fileTargets) files.target(id, target.bounds, target.receive);
    files.focus(focusedFileTarget);
    for (const pending of fileWaiters.splice(0)) pending.resolve(files);
  },
  fileTarget(id, bounds, receive) {
    fileTargets.set(id, { bounds, receive });
    browserFiles?.target(id, bounds, receive);
  },
  forgetFileTarget(id) {
    fileTargets.delete(id);
    browserFiles?.forget(id);
    if (focusedFileTarget === id) focusedFileTarget = "";
  },
  action(name) {
    if (name.startsWith("fileFocus:")) {
      focusedFileTarget = name.slice(10);
      browserFiles?.focus(focusedFileTarget);
    } else if (name.startsWith("fileRelease:")) browserFiles?.release(name.slice(12));
  },
  async invoke(name, payload) {
    return (await filesReady()).invoke(name, payload);
  },
  callback(payload) {
    return new Promise((resolve, reject) => {
      const requestId = ++callbackSequence;
      callbacks.set(requestId, { resolve, reject });
      parent.postMessage(
        { type: "zoo-compose-catalog-callback", session: callbackSession, requestId, payload },
        location.origin,
      );
    });
  },
  ready(next, dispose, handleEscape) {
    update = next;
    close = dispose;
    escape = handleEscape;
  },
  painted(defaultMessage) {
    clearTimeout(timeout);
    status.hidden = true;
    send("ready", defaultMessage);
  },
  scroll(top) {
    parent.postMessage({ type: "zoo-compose-catalog-scroll", top }, location.origin);
  },
  measured(width, height) {
    parent.postMessage({ type: "zoo-compose-catalog-size", width, height }, location.origin);
  },
  preference(action) {
    const [name, value] = action.split(":");
    parent.postMessage({ type: "zoo-compose-catalog-preference", name, value }, location.origin);
  },
  overlay(open) {
    parent.postMessage({ type: "zoo-compose-catalog-overlay", open }, location.origin);
  },
};
window.addEventListener("message", (event) => {
  if (
    event.origin === location.origin &&
    event.source === parent &&
    event.data?.type === "zoo-compose-catalog-callback-result" &&
    event.data.session === callbackSession
  ) {
    const pending = callbacks.get(event.data.requestId);
    if (pending) {
      callbacks.delete(event.data.requestId);
      if (event.data.ok) pending.resolve(event.data.result);
      else pending.reject(new Error(event.data.error));
    }
    return;
  }
  if (
    event.origin !== location.origin ||
    event.source !== parent ||
    event.data?.type !== "zoo-compose-catalog-options"
  )
    return;
  const options = { ...event.data.options, origin: location.origin };
  globalThis.zooCatalog.options = options;
  document.documentElement.lang = options.locale;
  if (!status.hidden)
    status.textContent = options.labels?.["experimental.catalogLoading"] ?? "Compose…";
  globalThis.__PARITY_OPTIONS__ = options;
  if (update) update(JSON.stringify(options));
  if (!started) {
    started = true;
    const script = document.createElement("script");
    script.type = "module";
    script.src = "/compose/zoo-compose.js";
    script.onerror = () => fail("Composeの配布物を取得できませんでした。");
    window.addEventListener(
      "zoo-compose-catalog-module-ready",
      () => globalThis.__zooComposeCatalogBoot(),
      { once: true },
    );
    document.head.append(script);
  }
});
const routeEscape = (event) => {
  if (
    event.key === "Escape" &&
    !event.isComposing &&
    event.keyCode !== 229 &&
    !event.defaultPrevented &&
    escape?.()
  ) {
    event.preventDefault();
    event.stopPropagation();
  }
};
window.addEventListener("keydown", routeEscape, true);
window.addEventListener(
  "pagehide",
  () => {
    window.removeEventListener("keydown", routeEscape, true);
    close?.();
    for (const pending of callbacks.values()) pending.reject(new Error("Catalog disposed"));
    callbacks.clear();
    browserFiles?.close();
    for (const pending of fileWaiters.splice(0)) pending.reject(new Error("Catalog disposed"));
    fileTargets.clear();
  },
  { once: true },
);
send("waiting");
