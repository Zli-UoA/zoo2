import { applyComposeInputHints } from "../../lib/compose/input-hints";
import { createComposeFiles } from "../../lib/compose/files";
import { useEffect, useRef, useState } from "react";
import { useRouter, useRouterState } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import { ApiError, buildAuthHeaders, createConfiguredApiClient } from "../../lib/api/client";
import { notifyAuthorized, notifyUnauthorized } from "../../lib/auth";
import { getCurrentUserStore } from "../../lib/currentUser";
import { resolveConfig, type AppConfig } from "../../lib/config";
import { switchFrontend } from "../../lib/experimentalFrontend";
import { createComposeCache, type ComposeCache } from "../../lib/compose/cache";
import { PREVIEW_USERS, PREVIEW_USER_KEY } from "../../lib/preview";
import { readLastOfflineUserId } from "../../lib/offline/identity";
import { openOfflinePersistence } from "../../lib/offline/persistence";
import { useOfflineStatus } from "../../lib/offline/offline-context";
import { useTheme } from "../theme/ThemeProvider";
import type { Theme } from "../theme/theme";
import { useLocale } from "../i18n/LocaleProvider";
import { isLocale } from "../../lib/i18n/locale";
import { OfflineProvider } from "../offline/OfflineProvider";
import { useVisualViewport } from "../layout/useVisualViewport";
import { getOidcClient } from "../../lib/oidc/session";
import { createBubbleExit } from "../../lib/compose/bubble-exit";
import {
  applicationServerKey,
  disablePush,
  registerWorker,
  saveSubscription,
  supportsPush,
} from "../../lib/pwa/push";

type Update = (environment: string) => void;
interface ComposeBridge {
  config: string;
  headers: () => Promise<string>;
  navigate: (path: string, replace?: boolean, state?: Record<string, unknown>) => void;
  fileTarget: (id: string, bounds: string, receive: (raw: string) => boolean) => void;
  forgetFileTarget: (id: string) => void;
  cache: ComposeCache;
  translate: (key: string, args: string) => string;
  format: (value: string, day: boolean, offset: number) => string;
  notificationTime: (value: string) => string;
  fullTimestamp: (value: string) => string;
  action: (name: string) => void;
  invoke: (name: string, payload: string) => Promise<string>;
  authStatus: (status: number) => void;
  ready: (update: Update, dispose: () => void, escape: () => boolean) => void;
  paintReady: () => void;
}
declare global {
  var __zooCompose: ComposeBridge | undefined;
  var __zooComposeBoot: (() => void) | undefined;
}
let moduleReady: Promise<void> | undefined;
function loadNativeModule(): Promise<void> {
  if (globalThis.__zooComposeBoot) return Promise.resolve();
  return (moduleReady ??= new Promise<void>((resolve, reject) => {
    const script = document.createElement("script");
    const timer = setTimeout(() => fail(), 90_000);
    const ready = () => {
      clearTimeout(timer);
      window.removeEventListener("zoo-compose-module-ready", ready);
      resolve();
    };
    const fail = () => {
      clearTimeout(timer);
      window.removeEventListener("zoo-compose-module-ready", ready);
      moduleReady = undefined;
      script.remove();
      reject(new Error("Compose module failed to load"));
    };
    window.addEventListener("zoo-compose-module-ready", ready);
    script.type = "module";
    script.src = "/compose/zoo-compose.js";
    script.onerror = fail;
    document.head.append(script);
  }));
}

type InstallEvent = Event & { prompt(): Promise<void>; userChoice: Promise<unknown> };

export function ComposeFrontend() {
  const [config] = useState(resolveConfig);
  return (
    <OfflineProvider
      config={config}
      collectionFactory={null}
      renderStatus={(phase, retry) => <ComposeRuntime config={config} status={{ phase, retry }} />}
    >
      <ComposeRuntime config={config} />
    </OfflineProvider>
  );
}
export interface NativeStatus {
  phase: string;
  message?: string;
  retry: () => void;
}
export function ComposeRuntime({ config, status }: { config: AppConfig; status?: NativeStatus }) {
  const router = useRouter();
  const location = useRouterState({ select: (state) => state.location.href });
  const historyState = useRouterState({ select: (state) => state.location.state });
  const { t } = useTranslation();
  const theme = useTheme();
  const language = useLocale();
  const offline = useOfflineStatus();
  const viewportRef = useVisualViewport();
  const [error, setError] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [cacheError, setCacheError] = useState<string | null>(null);
  const updateRef = useRef<Update | null>(null);
  const escapeRef = useRef<(() => boolean) | null>(null);
  const stateRef = useRef({
    location,
    historyState,
    theme,
    language,
    t,
    offline,
    status,
    cacheError,
  });
  stateRef.current = { location, historyState, theme, language, t, offline, status, cacheError };
  const environment = () =>
    JSON.stringify({
      path: stateRef.current.location,
      historyState: stateRef.current.historyState,
      theme: stateRef.current.theme.theme,
      dark: stateRef.current.theme.resolvedTheme === "dark",
      online: navigator.onLine,
      visible: document.visibilityState === "visible",
      locale: stateRef.current.language.locale,
      mac: /Mac|iPhone|iPad/.test(navigator.userAgent),
      touch: navigator.maxTouchPoints > 0,
      oidc: Boolean(config.oidc),
      development: import.meta.env.DEV,
      focused: document.hasFocus(),
      persistenceStatus:
        stateRef.current.cacheError !== null ? "error" : stateRef.current.offline.persistenceStatus,
      persistenceError:
        stateRef.current.cacheError ?? stateRef.current.offline.persistenceError ?? "",
      reducedMotion: window.matchMedia("(prefers-reduced-motion: reduce)").matches,
      status: stateRef.current.status?.phase ?? "",
      statusMessage: stateRef.current.status?.message ?? "",
    });
  useEffect(() => {
    let cancelled = false;
    let dispose: (() => void) | undefined;
    let clearInputHints: (() => void) | undefined;
    let root: HTMLDivElement | undefined;
    let ownBridge: ComposeBridge | undefined;
    const readyTimeout = setTimeout(() => {
      if (!cancelled) setError(true);
    }, 90_000);
    let install: InstallEvent | null = null;
    const fileBrowser = createComposeFiles(viewportRef.current!);
    const publicRequests = new Map<string, AbortController>();
    const bubbleExit = createBubbleExit(
      new URL(`${config.apiBaseUrl.replace(/\/$/, "")}/api/bubble`, window.location.origin).href,
    );
    const visibilityChanged = () => update();
    const onInstall = (event: Event) => {
      event.preventDefault();
      install = event as InstallEvent;
    };
    const installed = () => {
      install = null;
    };
    const update = () => updateRef.current?.(environment());
    const start = async () => {
      const user = getCurrentUserStore(config).getState();
      const userId =
        user.currentUserId ??
        (!navigator.onLine ? readLastOfflineUserId() : null) ??
        config.devUserId ??
        "anonymous";
      const database =
        offline.persistenceStatus === "ready"
          ? (await openOfflinePersistence(offline.namespace)).database
          : null;
      if (cancelled) return;
      const bridge: ComposeBridge = {
        config: JSON.stringify({
          ...config,
          apiBaseUrl: new URL(config.apiBaseUrl || "/", window.location.origin).href.replace(
            /\/$/,
            "",
          ),
          assetsBaseUrl: window.location.origin,
          userId,
          isAdmin: user.isAdmin,
          previewUsers:
            import.meta.env.VITE_PREVIEW_AUTH === "true" && !config.oidc ? PREVIEW_USERS : [],
          environment: environment(),
        }),
        cache: createComposeCache(database, (cause) => {
          if (!cancelled) setCacheError(cause instanceof Error ? cause.message : String(cause));
        }),
        fileTarget: (id, bounds, receive) => fileBrowser.target(id, bounds, receive),
        forgetFileTarget: (id) => fileBrowser.forget(id),
        headers: async () => {
          if (cancelled) throw new Error("Compose frontend was disposed");
          return JSON.stringify(
            Object.fromEntries(await buildAuthHeaders({ devUserId: config.devUserId })),
          );
        },
        navigate: (path, replace = false, state) => {
          if (path.startsWith("/") && !path.startsWith("//")) {
            if (replace) router.history.replace(path, state);
            else router.history.push(path, state);
            void router.load().catch(() => setError(true));
          }
        },
        translate: (key, args) => {
          const values = JSON.parse(args) as Record<string, unknown>;
          if (typeof values.count === "string") values.count = Number(values.count);
          return String(stateRef.current.t(key, { defaultValue: key, ...values }));
        },
        fullTimestamp: (value) =>
          new Date(
            /[Z+-]\d|Z$/.test(value.slice(10)) ? value : `${value.replace(" ", "T")}Z`,
          ).toLocaleString(stateRef.current.language.locale),
        notificationTime: (value) => {
          const date = new Date(
            /[Z+-]\d|Z$/.test(value.slice(10)) ? value : `${value.replace(" ", "T")}Z`,
          );
          return Number.isNaN(date.getTime())
            ? value
            : new Intl.DateTimeFormat(stateRef.current.language.locale, {
                month: "short",
                day: "numeric",
                hour: "2-digit",
                minute: "2-digit",
                hourCycle: "h23",
              }).format(date);
        },
        format: (value, day, offset) => {
          const date = new Date(
            /[Z+-]\d|Z$/.test(value.slice(10)) ? value : `${value.replace(" ", "T")}Z`,
          );
          if (offset) date.setDate(date.getDate() + offset);
          return Number.isNaN(+date)
            ? value
            : new Intl.DateTimeFormat(
                stateRef.current.language.locale,
                day
                  ? { year: "numeric", month: "long", day: "numeric", weekday: "short" }
                  : { hour: "2-digit", minute: "2-digit", hourCycle: "h23" },
              ).format(date);
        },
        action: (name) => {
          if (name === "back") {
            if ((window.history.state?.__TSR_index ?? 0) > 0) router.history.back();
            else {
              router.history.replace("/");
              void router.load().catch(() => setError(true));
            }
            return;
          }
          if (name.startsWith("fileFocus:")) fileBrowser.focus(name.slice(10));
          if (name.startsWith("fileRelease:")) fileBrowser.release(name.slice(12));
          if (name.startsWith("mfm:"))
            window.dispatchEvent(new CustomEvent("mfm-click", { detail: name.slice(4) }));
          else if (name === "retry") stateRef.current.status?.retry();
          else if (name === "retryOffline") {
            setCacheError(null);
            if (navigator.onLine) stateRef.current.offline.retryPersistence();
            else window.location.reload();
          } else if (name === "react") switchFrontend("react");
          else if (name.startsWith("theme:")) {
            const mode = name.slice(6);
            if (["light", "dark", "system"].includes(mode))
              stateRef.current.theme.setTheme(mode as Theme);
          } else if (name.startsWith("locale:")) {
            const locale = name.slice(7);
            if (isLocale(locale)) stateRef.current.language.setLocale(locale);
          } else if (name.startsWith("open:")) {
            const url = new URL(name.slice(5), window.location.origin);
            if (["http:", "https:"].includes(url.protocol))
              window.open(url.href, "_blank", "noopener,noreferrer");
          }
        },
        invoke: async (name, payload) => {
          if (["pickFiles", "fileRead", "filePut", "attachmentDownload"].includes(name))
            return fileBrowser.invoke(name, payload);
          if (name === "bubbleExit") bubbleExit.update(payload);
          if (name === "cancelPublic") publicRequests.get(payload)?.abort();
          if (name === "publicGet" || name === "publicImage") {
            if (cancelled) throw new Error("Compose frontend was disposed");
            const { requestId, payload: requestPayload } = JSON.parse(payload) as {
              requestId: string;
              payload: string;
            };
            const raw =
              name === "publicImage"
                ? requestPayload
                : (JSON.parse(requestPayload) as { url: string }).url;
            const url = new URL(raw, window.location.origin);
            if (!["https:", "http:"].includes(url.protocol) || url.username || url.password)
              throw new Error("Unsupported public URL");
            const controller = new AbortController();
            publicRequests.set(requestId, controller);
            try {
              const response = await fetch(url, {
                signal: AbortSignal.any([controller.signal, AbortSignal.timeout(5000)]),
                credentials: "omit",
                referrerPolicy: "no-referrer",
                headers: {
                  Accept:
                    name === "publicImage"
                      ? "image/png, image/jpeg, image/webp, image/*"
                      : "application/json, text/plain",
                },
              });
              if (!response.ok) throw new Error(`Public request failed: ${response.status}`);
              if (name === "publicImage") {
                const bytes = new Uint8Array(await response.arrayBuffer());
                if (bytes.byteLength > 50 * 1024 * 1024) throw new Error("Public image too large");
                let data = "";
                for (let offset = 0; offset < bytes.length; offset += 32768)
                  data += String.fromCharCode(...bytes.subarray(offset, offset + 32768));
                return btoa(data);
              }
              const body = await response.text();
              if (body.length > 65536) throw new Error("Public response too large");
              return JSON.stringify({
                body,
                contentType: response.headers.get("content-type") ?? "",
              });
            } finally {
              publicRequests.delete(requestId);
            }
          }
          if (name === "redirect") {
            const url = new URL(payload);
            if (["http:", "https:"].includes(url.protocol)) window.location.assign(url.href);
          }
          if (
            name === "previewUser" &&
            import.meta.env.VITE_PREVIEW_AUTH === "true" &&
            !config.oidc
          ) {
            if (PREVIEW_USERS.some((user) => user.id === payload)) {
              try {
                window.sessionStorage.setItem(PREVIEW_USER_KEY, payload);
                window.location.reload();
              } catch {
                /* Keep the user that requests still use. */
              }
            }
          }
          if (name === "refreshUser") {
            await getCurrentUserStore(config).refresh();
            window.location.reload();
          }
          if (name === "copy") await navigator.clipboard.writeText(payload);
          if (name === "install" && install) {
            await install.prompt();
            await install.userChoice;
            install = null;
          }
          if (name === "logout") {
            await disablePush(config);
            dispose?.();
            try {
              await stateRef.current.offline.clearUserData();
            } catch {
              /* Identity is forgotten and deletion is retried on the next startup. */
            }
            await getOidcClient(config)?.logout();
          }
          if (name === "storageRead") {
            try {
              return localStorage.getItem(`zoo-compose:${userId}:${payload}`) ?? "";
            } catch {
              return "";
            }
          }
          if (name === "storageWrite") {
            const { key, value } = JSON.parse(payload) as { key: string; value: string };
            try {
              localStorage.setItem(`zoo-compose:${userId}:${key}`, value);
            } catch {
              /* Storage may be disabled or full. Keep the current draft in Kotlin state. */
            }
          }
          if (name === "capabilities" || name === "push") {
            const supported = supportsPush();
            const worker =
              window.isSecureContext && "serviceWorker" in navigator
                ? await registerWorker()
                : null;
            const settings = supported
              ? await createConfiguredApiClient(config).get<{ publicKey: string | null }>(
                  "/api/push/config",
                )
              : { publicKey: null };
            let subscription = await worker?.pushManager?.getSubscription();
            if (name === "capabilities" && subscription && settings.publicKey) {
              try {
                await saveSubscription(config, subscription);
              } catch (cause) {
                if (!(cause instanceof ApiError) || cause.status !== 409) throw cause;
                if (!(await subscription.unsubscribe()))
                  throw new Error("Could not revoke push subscription");
                subscription = null;
              }
            }
            if (name === "push" && supported && settings.publicKey && worker) {
              if (subscription) {
                await disablePush(config);
                subscription = null;
              } else if ((await Notification.requestPermission()) === "granted") {
                subscription = await worker.pushManager.subscribe({
                  userVisibleOnly: true,
                  applicationServerKey: applicationServerKey(settings.publicKey),
                });
                try {
                  await saveSubscription(config, subscription);
                } catch (cause) {
                  await subscription.unsubscribe();
                  throw cause;
                }
              }
            }
            return JSON.stringify({
              installAvailable: Boolean(install),
              pushSupported: supported,
              pushConfigured: Boolean(settings.publicKey),
              pushEnabled: Boolean(subscription),
            });
          }
          return "{}";
        },
        authStatus: (status) => {
          if (!cancelled) {
            if (status === 401) notifyUnauthorized();
            else if (status >= 200 && status < 300) notifyAuthorized();
          }
        },
        paintReady: () => {
          if (!cancelled) {
            clearTimeout(readyTimeout);
            setLoaded(true);
          }
        },
        ready: (nextUpdate, nextDispose, escape) => {
          if (cancelled) {
            nextDispose();
            return;
          }
          updateRef.current = nextUpdate;
          escapeRef.current = escape;
          dispose = nextDispose;
          update();
        },
      };
      await loadNativeModule();
      if (cancelled) return;
      root = document.createElement("div");
      root.id = "zoo-compose-root";
      clearInputHints = applyComposeInputHints(root);
      root.style.width = "100%";
      root.style.height = "100%";
      document.getElementById("zoo-compose-host")?.append(root);
      ownBridge = bridge;
      globalThis.__zooCompose = bridge;
      globalThis.__zooComposeBoot!();
      /* module boot is synchronous; Kotlin registers its update/dispose callbacks here. */
    };
    void start().catch(() => {
      if (!cancelled) setError(true);
    });
    window.addEventListener("beforeinstallprompt", onInstall);
    window.addEventListener("appinstalled", installed);
    window.addEventListener("online", update);
    window.addEventListener("focus", update);
    window.addEventListener("blur", update);
    window.addEventListener("offline", update);
    document.addEventListener("visibilitychange", visibilityChanged);
    const escape = (event: KeyboardEvent) => {
      if (
        event.key === "Escape" &&
        !event.isComposing &&
        event.keyCode !== 229 &&
        !event.defaultPrevented &&
        escapeRef.current?.()
      ) {
        event.preventDefault();
        event.stopPropagation();
      }
    };
    // Route modal Escape before a backing editor or a disposed focus target consumes it.
    window.addEventListener("keydown", escape, true);
    const resize = new ResizeObserver(() => window.dispatchEvent(new Event("resize")));
    if (viewportRef.current) resize.observe(viewportRef.current);
    return () => {
      cancelled = true;
      fileBrowser.close();
      publicRequests.forEach((controller) => controller.abort());
      bubbleExit.close();
      clearTimeout(readyTimeout);
      resize.disconnect();
      updateRef.current = null;
      escapeRef.current = null;
      window.removeEventListener("keydown", escape, true);
      dispose?.();
      clearInputHints?.();
      root?.remove();
      if (globalThis.__zooCompose === ownBridge) delete globalThis.__zooCompose;
      window.removeEventListener("beforeinstallprompt", onInstall);
      window.removeEventListener("appinstalled", installed);
      window.removeEventListener("online", update);
      window.removeEventListener("focus", update);
      window.removeEventListener("blur", update);
      window.removeEventListener("offline", update);
      document.removeEventListener("visibilitychange", visibilityChanged);
    };
  }, [config, offline.namespace, offline.persistenceStatus, router]);
  useEffect(() => {
    updateRef.current?.(environment());
  }, [
    location,
    historyState,
    theme.resolvedTheme,
    theme.theme,
    language.locale,
    status?.phase,
    status?.message,
    offline.persistenceError,
    cacheError,
  ]);
  return (
    <div
      ref={viewportRef}
      className="fixed inset-x-0 top-0 h-dvh w-full overflow-hidden bg-background text-foreground"
      data-app-shell
      data-compose-shell
    >
      <div id="zoo-compose-host" className="h-full w-full" />
      {!loaded || error ? (
        <div
          className="fixed inset-0 z-50 flex flex-col items-center justify-center gap-4 bg-background p-6"
          role={error ? "alert" : "status"}
        >
          <p>{t(error ? "experimental.loadError" : "common.loading")}</p>
          <button onClick={() => switchFrontend("react")}>{t("experimental.useReact")}</button>
        </div>
      ) : null}
    </div>
  );
}
