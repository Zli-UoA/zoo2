// Each story owns an iframe: leaving the story destroys its Canvas, input owner and font tasks.
const status = document.getElementById("status");
const send = (state, defaultMessage) =>
  parent.postMessage(
    { type: "zoo-compose-catalog-status", state, defaultMessage },
    location.origin,
  );
let update;
let close;
let started = false;
const timeout = setTimeout(() => fail("Composeの読み込みがタイムアウトしました。"), 90_000);
function fail(message) {
  const labels = globalThis.zooCatalog.options.labels ?? {};
  clearTimeout(timeout);
  status.textContent = `${labels["experimental.catalogLoadError"] ?? message} ${labels["experimental.catalogBuildHint"] ?? "Install JDK 21 and run pnpm build:compose in frontend."}`;
  send("error");
}
globalThis.zooCatalog = {
  options: {},
  ready(next, dispose) {
    update = next;
    close = dispose;
  },
  painted(defaultMessage) {
    clearTimeout(timeout);
    status.hidden = true;
    send("ready", defaultMessage);
  },
};
window.addEventListener("message", (event) => {
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
window.addEventListener("pagehide", () => close?.(), { once: true });
send("waiting");
