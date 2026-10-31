import { useEffect, useRef, useState } from "react";
import { resources } from "../src/lib/i18n";
import type { Locale } from "../src/lib/i18n/locale";

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
    | "app-blocks";
  fixture?: Record<string, unknown>;
  path?: string;
  content: string;
  disabled: boolean;
  grouped: boolean;
  dark: boolean;
  locale: Locale;
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
  const frame = useRef<HTMLIFrameElement>(null);
  const [state, setState] = useState("loading");
  const options = { ...props, labels: labels[props.locale], locales: labels };
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
    const receive = (event: MessageEvent) => {
      if (
        event.origin !== window.location.origin ||
        event.source !== frame.current?.contentWindow ||
        event.data?.type !== "zoo-compose-catalog-status"
      )
        return;
      setState(event.data.state);
      if (event.data.state === "waiting") send();
    };
    window.addEventListener("message", receive);
    return () => window.removeEventListener("message", receive);
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
  ]);
  return (
    <iframe
      ref={frame}
      title="Kotlin component preview"
      src="/compose-catalog.html"
      onLoad={send}
      data-compose-catalog-state={state}
      style={{ display: "block", border: 0, width: "100%", height: "100dvh", minHeight: 480 }}
    />
  );
}
