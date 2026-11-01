import { createComposeFiles } from "../src/lib/compose/files";
import { detectComposerPlatform } from "../src/components/chat/messages/Composer";
import { useMediaQuery } from "../src/components/layout/useMediaQuery";
import { applyComposeInputHints } from "../src/lib/compose/input-hints";
import { selectedFrontend, switchFrontend } from "../src/lib/experimentalFrontend";
import { dispatchStoryCallback, type StoryCallbacks } from "./story-callbacks";
import { useEffect, useRef, useState } from "react";
import { resources } from "../src/lib/i18n";
import type { Locale } from "../src/lib/i18n/locale";
import { isLocale } from "../src/lib/i18n/locale";
import { isTheme } from "../src/components/theme/theme";
import { useTheme } from "../src/components/theme/ThemeProvider";
import { useLocale } from "../src/components/i18n/LocaleProvider";

export interface KotlinPreviewProps {
  sample:
    | "message"
    | "composer"
    | "emoji-picker"
    | "rich-text"
    | "link-preview"
    | "settings"
    | "settings-members"
    | "bubble"
    | "notifications"
    | "notifications-loading"
    | "card"
    | "app-blocks"
    | "button"
    | "input"
    | "textarea"
    | "avatar"
    | "skeleton"
    | "select"
    | "control-panel"
    | "dialog-demo"
    | "confirm"
    | "primitive-story"
    | "message-story"
    | "shell-story"
    | "app-story"
    | "preference-story"
    | "notification-story"
    | "home-story";
  fixture?: Record<string, unknown>;
  path?: string;
  content: string;
  disabled: boolean;
  grouped: boolean;
  dark: boolean;
  locale: Locale;
  /** The same surface class used by the React story decorator. */
  surfaceClassName?: string;
  fullHeight?: boolean;
  /** Padding owned by Storybook around a viewport-sized preview. */
  viewportHeightInset?: number;
  intrinsicWidth?: number;
  measureWidth?: boolean;
  callbacks?: StoryCallbacks;
}

function flatten(value: object, prefix = ""): Record<string, string> {
  return Object.fromEntries(
    Object.entries(value).flatMap(([key, item]) => {
      const path = prefix ? `${prefix}.${key}` : key;
      return typeof item === "string" ? [[path, item]] : Object.entries(flatten(item, path));
    }),
  );
}
const labels = Object.fromEntries(
  Object.entries(resources).map(([locale, resource]) => [locale, flatten(resource.translation)]),
);

export function KotlinPreview(props: KotlinPreviewProps) {
  const { theme, resolvedTheme, setTheme } = useTheme();
  const { locale, setLocale } = useLocale();
  const frame = useRef<HTMLIFrameElement>(null);
  const touch = useMediaQuery("(hover: none)");
  const [mac] = useState(() => detectComposerPlatform() === "mac");
  const [environment, setEnvironment] = useState(() => ({
    touch,
    mac,
    viewportWidth: window.innerWidth,
    viewportHeight: window.innerHeight,
    visualViewportHeight:
      window.visualViewport?.scale === 1 ? window.visualViewport.height : window.innerHeight,
    visualViewportTop:
      window.visualViewport?.scale === 1 && window.innerHeight - window.visualViewport.height >= 1
        ? window.visualViewport.offsetTop
        : 0,
    focused: document.hasFocus(),
    visible: document.visibilityState === "visible",
    reducedMotion: window.matchMedia("(prefers-reduced-motion: reduce)").matches,
  }));
  useEffect(() => {
    const media = window.matchMedia("(prefers-reduced-motion: reduce)");
    const update = () =>
      setEnvironment((previous) => {
        const next = {
          touch,
          mac,
          viewportWidth: window.innerWidth,
          viewportHeight: window.innerHeight,
          visualViewportHeight:
            window.visualViewport?.scale === 1 ? window.visualViewport.height : window.innerHeight,
          visualViewportTop:
            window.visualViewport?.scale === 1 &&
            window.innerHeight - window.visualViewport.height >= 1
              ? window.visualViewport.offsetTop
              : 0,
          focused: document.hasFocus(),
          visible: document.visibilityState === "visible",
          reducedMotion: media.matches,
        };
        return JSON.stringify(previous) === JSON.stringify(next) ? previous : next;
      });
    update();
    for (const event of ["resize", "focus", "blur"]) window.addEventListener(event, update);
    window.visualViewport?.addEventListener("resize", update);
    window.visualViewport?.addEventListener("scroll", update);
    document.addEventListener("visibilitychange", update);
    media.addEventListener("change", update);
    return () => {
      for (const event of ["resize", "focus", "blur"]) window.removeEventListener(event, update);
      window.visualViewport?.removeEventListener("resize", update);
      window.visualViewport?.removeEventListener("scroll", update);
      document.removeEventListener("visibilitychange", update);
      media.removeEventListener("change", update);
    };
  }, [touch, mac]);

  const [state, setState] = useState("loading");
  const [height, setHeight] = useState(480);
  const [naturalWidth, setNaturalWidth] = useState<number | null>(null);
  const placeholder = useRef<HTMLDivElement>(null);
  const [overlayBounds, setOverlayBounds] = useState<{
    x: number;
    y: number;
    width: number;
    height: number;
  } | null>(null);
  const [normalBounds, setNormalBounds] = useState<{
    x: number;
    y: number;
    width: number;
    height: number;
  } | null>(null);
  const fixedHeight = /(?:^|\s)h-\[(\d+)px\]/.exec(props.surfaceClassName ?? "")?.[1];
  const { callbacks, ...serializable } = props;
  const callbackRef = useRef(callbacks ?? {});
  callbackRef.current = callbacks ?? {};
  const options = {
    ...serializable,
    ...environment,
    frontend: selectedFrontend(),
    overlayBounds,
    normalBounds,
    theme,
    dark: resolvedTheme === "dark",
    locale,
    labels: labels[locale],
    locales: labels,
  };
  const preferences = useRef({ setTheme, setLocale });
  preferences.current = { setTheme, setLocale };
  const current = useRef(options);
  current.current = options;
  const send = () =>
    frame.current?.contentWindow?.postMessage(
      {
        type: "zoo-compose-catalog-options",
        options: current.current,
      },
      window.location.origin,
    );
  useEffect(() => {
    let ownedFiles: ReturnType<typeof createComposeFiles> | undefined;
    let fileDocument: Document | undefined;
    const connectFiles = () => {
      const target = frame.current?.contentWindow as
        | (Window & {
            zooCatalog?: { installFiles: (files: ReturnType<typeof createComposeFiles>) => void };
          })
        | null;
      const owner = frame.current?.contentDocument;
      const viewport = owner?.getElementById("zoo-compose-root");
      if (!owner || !viewport || !target?.zooCatalog?.installFiles || owner === fileDocument)
        return;
      ownedFiles?.close();
      ownedFiles = createComposeFiles(viewport);
      fileDocument = owner;
      target.zooCatalog.installFiles(ownedFiles);
    };
    const receive = (event: MessageEvent) => {
      if (
        event.origin !== window.location.origin ||
        event.source !== frame.current?.contentWindow ||
        ![
          "zoo-compose-catalog-callback",
          "zoo-compose-catalog-status",
          "zoo-compose-catalog-size",
          "zoo-compose-catalog-scroll",
          "zoo-compose-catalog-preference",
          "zoo-compose-catalog-overlay",
        ].includes(event.data?.type)
      )
        return;
      if (event.data.type === "zoo-compose-catalog-callback") {
        void dispatchStoryCallback(callbackRef.current, event.data, frame.current!.contentWindow!);
        return;
      }
      if (event.data.type === "zoo-compose-catalog-scroll") {
        const top: unknown = event.data.top;
        if (typeof top === "number" && Number.isFinite(top) && top >= 0 && placeholder.current)
          window.scrollTo({
            top: window.scrollY + placeholder.current.getBoundingClientRect().top + top,
          });
        return;
      }
      if (event.data.type === "zoo-compose-catalog-overlay") {
        if (event.data.open === true && placeholder.current) {
          const rect = placeholder.current.getBoundingClientRect();
          setOverlayBounds((previous) =>
            previous?.x === rect.x &&
            previous.y === rect.y &&
            previous.width === rect.width &&
            previous.height === rect.height
              ? previous
              : { x: rect.x, y: rect.y, width: rect.width, height: rect.height },
          );
        } else if (event.data.open === false) setOverlayBounds(null);
        return;
      }
      if (event.data.type === "zoo-compose-catalog-preference") {
        if (
          event.data.name === "frontend" &&
          (event.data.value === "react" || event.data.value === "compose")
        )
          switchFrontend(event.data.value);
        if (event.data.name === "theme" && isTheme(event.data.value))
          preferences.current.setTheme(event.data.value);
        if (event.data.name === "locale" && isLocale(event.data.value))
          preferences.current.setLocale(event.data.value);
        return;
      }
      if (event.data.type === "zoo-compose-catalog-size") {
        if (
          typeof event.data.width === "number" &&
          Number.isFinite(event.data.width) &&
          event.data.width >= 0
        )
          setNaturalWidth(event.data.width);
        if (
          typeof event.data.height === "number" &&
          Number.isFinite(event.data.height) &&
          event.data.height >= 0
        )
          setHeight(event.data.height);
        return;
      }
      if (event.data.type === "zoo-compose-catalog-status") connectFiles();
      setState(event.data.state);
      if (event.data.state === "waiting") send();
    };
    window.addEventListener("message", receive);
    return () => {
      window.removeEventListener("message", receive);
      ownedFiles?.close();
    };
  }, []);
  useEffect(() => {
    if (state !== "ready") return;
    const owner = frame.current?.contentDocument;
    if (owner) return applyComposeInputHints(owner);
  }, [state]);
  const expanded = overlayBounds !== null;
  useEffect(() => {
    if (!expanded || !placeholder.current) return;
    const measure = () => {
      const element = placeholder.current;
      if (!element) return;
      const rect = element.getBoundingClientRect();
      setOverlayBounds((previous) =>
        previous &&
        (previous.x !== rect.x ||
          previous.y !== rect.y ||
          previous.width !== rect.width ||
          previous.height !== rect.height)
          ? { x: rect.x, y: rect.y, width: rect.width, height: rect.height }
          : previous,
      );
    };
    const observer = new ResizeObserver(measure);
    observer.observe(placeholder.current);
    window.addEventListener("resize", measure);
    window.addEventListener("scroll", measure, true);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", measure);
      window.removeEventListener("scroll", measure, true);
    };
  }, [expanded]);
  useEffect(() => {
    if (!placeholder.current) return;
    const measure = () => {
      const element = placeholder.current;
      if (!element) return;
      const rect = element.getBoundingClientRect();
      const next = { x: rect.x, y: rect.y, width: rect.width, height: rect.height };
      setNormalBounds((previous) =>
        previous?.x === next.x &&
        previous.y === next.y &&
        previous.width === next.width &&
        previous.height === next.height
          ? previous
          : next,
      );
    };
    const observer = new ResizeObserver(measure);
    observer.observe(placeholder.current);
    window.addEventListener("resize", measure);
    window.addEventListener("scroll", measure, true);
    measure();
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", measure);
      window.removeEventListener("scroll", measure, true);
    };
  }, []);
  useEffect(send, [
    props.path,
    props.fixture,
    props.sample,
    props.content,
    props.disabled,
    props.grouped,
    props.dark,
    props.locale,
    props.fullHeight,
    theme,
    resolvedTheme,
    locale,
    overlayBounds,
    normalBounds,
    environment,
  ]);
  const normalHeight = fixedHeight
    ? Number(fixedHeight)
    : props.fullHeight
      ? `calc(100dvh - ${props.viewportHeightInset ?? 0}px)`
      : height;
  const normalClip = normalBounds
    ? `inset(${Math.max(0, normalBounds.y - 8)}px ${Math.max(0, environment.viewportWidth - normalBounds.x - normalBounds.width - 8)}px ${Math.max(0, environment.viewportHeight - normalBounds.y - normalBounds.height - 8)}px ${Math.max(0, normalBounds.x - 8)}px)`
    : "inset(100%)";
  return (
    <div className={props.surfaceClassName}>
      <div
        ref={placeholder}
        style={{
          width:
            props.intrinsicWidth ??
            (props.measureWidth ? (naturalWidth ?? environment.viewportWidth) : "100%"),
          height: normalHeight,
          position: "relative",
        }}
      >
        <iframe
          ref={frame}
          title="Kotlin component preview"
          src="/compose-catalog.html"
          onLoad={send}
          data-compose-catalog-state={state}
          data-compose-catalog-interactions={JSON.stringify(props.fixture?.interactions ?? [])}
          style={{
            display: "block",
            border: 0,
            width: "100vw",
            height: "100dvh",
            position: "fixed",
            left: 0,
            top: 0,
            clipPath: overlayBounds ? "none" : normalClip,
            zIndex: overlayBounds ? 1000 : undefined,
          }}
        />
      </div>
    </div>
  );
}
