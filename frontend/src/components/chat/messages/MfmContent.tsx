import { Fragment, useEffect, useMemo, useState, type ReactNode } from "react";
import { Link as LinkIcon } from "lucide-react";
import { parse, toString, type MfmNode } from "mfm-js";
import katex from "katex";
import hljs from "highlight.js/lib/common";
import shortcodeData from "../../../lib/emoji/shortcodes.json";
import { ChannelHashtag } from "./MfmChannels";
import { messageTextSegments } from "./messageFormat";
import { functionPresentation, MFM_FUNCTIONS } from "./mfmFunctions";

export interface MfmContentProps {
  content: string;
  /** Registered custom emoji: name -> same-origin image URL. Unknown names stay readable. */
  emojiUrls?: Readonly<Record<string, string>>;
  /** Application event, never evaluated as JavaScript. */
  onClickEvent?: (event: string) => void;
}
const shortcodes: Readonly<Record<string, string>> = shortcodeData;
const linkClass =
  "rounded-sm text-brand underline underline-offset-2 outline-none focus-visible:ring-2 focus-visible:ring-ring/50";
function externalLink(url: string, children: ReactNode, rawUrl = false): ReactNode {
  return (
    <a
      href={url}
      target="_blank"
      rel="noopener noreferrer"
      className={
        rawUrl
          ? "inline-flex max-w-full items-baseline gap-1 rounded bg-sky-100 px-1 py-0.5 align-bottom text-sky-700 outline-none hover:bg-sky-200 focus-visible:ring-2 focus-visible:ring-ring/50 dark:bg-sky-950 dark:text-sky-300 dark:hover:bg-sky-900"
          : linkClass
      }
    >
      {rawUrl ? <LinkIcon aria-hidden="true" className="size-[1em] shrink-0" /> : null}
      {rawUrl ? <span className="min-w-0 [overflow-wrap:anywhere]">{children}</span> : children}
    </a>
  );
}
function text(content: string): ReactNode {
  return messageTextSegments(content).map((segment, index) => (
    <Fragment key={index}>
      {segment.kind === "link" ? (
        externalLink(segment.value, segment.value.replace(/^https?:\/\//i, ""), true)
      ) : segment.kind === "mention" ? (
        <span className="rounded bg-brand-muted px-0.5 font-medium text-brand-muted-foreground">
          {segment.value}
        </span>
      ) : (
        segment.value
      )}
    </Fragment>
  ));
}

// Custom emoji live in an operator-managed same-origin registry, not on an arbitrary remote host.
let registryRequest: Promise<Readonly<Record<string, string>>> | undefined;
function loadRegistry() {
  registryRequest ??= fetch("/mfm-emojis.json")
    .then(async (response) => {
      if (!response.ok) return {};
      const value: unknown = await response.json();
      if (!value || typeof value !== "object" || Array.isArray(value)) return {};
      return Object.fromEntries(
        Object.entries(value).filter(
          ([name, url]) =>
            /^[\w+-]+$/.test(name) && typeof url === "string" && /^\/(?!\/)/.test(url),
        ),
      );
    })
    .catch(() => ({}));
  return registryRequest;
}
function EmojiCode({
  name,
  emojiUrls,
}: {
  name: string;
  emojiUrls?: MfmContentProps["emojiUrls"];
}) {
  const [registry, setRegistry] = useState<Readonly<Record<string, string>>>({});
  useEffect(() => {
    if (emojiUrls) return;
    let active = true;
    void loadRegistry().then((value) => {
      if (active) setRegistry(value);
    });
    return () => {
      active = false;
    };
  }, [emojiUrls]);
  const urls = emojiUrls ?? registry;
  const url = Object.hasOwn(urls, name) ? urls[name] : undefined;
  if (url && /^\/(?!\/)/.test(url))
    return <img className="mfm-custom-emoji" src={url} alt={`:${name}:`} loading="lazy" />;
  return Object.hasOwn(shortcodes, name) ? shortcodes[name] : `:${name}:`;
}
function MathFormula({ formula, block }: { formula: string; block: boolean }) {
  const html = useMemo(
    () =>
      katex.renderToString(formula, {
        displayMode: block,
        throwOnError: false,
        trust: false,
        strict: "ignore",
        maxExpand: 1000,
        maxSize: 20,
        output: "htmlAndMathml",
      }),
    [formula, block],
  );
  // KaTeX is the sole producer of this HTML; trust:false disables HTML/URL extensions.
  return (
    <span
      className={block ? "mfm-math-block" : "mfm-math"}
      dangerouslySetInnerHTML={{ __html: html }}
    />
  );
}
function CodeBlock({ code, lang }: { code: string; lang: string | null }) {
  const html = useMemo(
    () =>
      lang && hljs.getLanguage(lang)
        ? hljs.highlight(code, { language: lang, ignoreIllegals: true }).value
        : null,
    [code, lang],
  );
  if (html === null)
    return (
      <pre className="mfm-code">
        <code>{code}</code>
      </pre>
    );
  // highlight.js escapes the source before adding its own span elements.
  return (
    <pre className="mfm-code">
      <code className="hljs" dangerouslySetInnerHTML={{ __html: html }} />
    </pre>
  );
}

function renderNode(
  node: MfmNode,
  options: Omit<MfmContentProps, "content">,
  inLink = false,
  scale = 1,
): ReactNode {
  const render = (nodes: MfmNode[], nextScale = scale) =>
    nodes.map((child, index) => (
      <Fragment key={index}>{renderNode(child, options, inLink, nextScale)}</Fragment>
    ));
  const children = () => render(node.children ?? []);
  switch (node.type) {
    case "text":
      return inLink ? node.props.text : text(node.props.text);
    case "unicodeEmoji":
      return node.props.emoji;
    case "emojiCode":
      return <EmojiCode name={node.props.name} emojiUrls={options.emojiUrls} />;
    case "bold":
      return <strong>{children()}</strong>;
    case "italic":
      return <em>{children()}</em>;
    case "strike":
      return <del>{children()}</del>;
    case "small":
      return <small>{children()}</small>;
    case "inlineCode":
      return <code className="rounded bg-muted px-1">{node.props.code}</code>;
    case "blockCode":
      return <CodeBlock {...node.props} />;
    case "plain":
      return node.children.map((child) => child.props.text).join("");
    case "quote":
      return (
        <blockquote className="border-l-2 border-muted-foreground pl-3">{children()}</blockquote>
      );
    case "center":
      return <div className="text-center">{children()}</div>;
    case "mathInline":
    case "mathBlock":
      return <MathFormula formula={node.props.formula} block={node.type === "mathBlock"} />;
    case "url":
      return inLink ? node.props.url : text(node.props.url);
    case "link": {
      if (inLink) return children();
      const label = node.children.map((child, index) => (
        <Fragment key={index}>{renderNode(child, options, true, scale)}</Fragment>
      ));
      return /^https?:\/\//i.test(node.props.url)
        ? externalLink(node.props.url, label)
        : toString(node);
    }
    case "mention": {
      // Local mentions retain the chat's existing styling; federated mentions link to their origin.
      const host = node.props.host;
      return !inLink && host && /^[a-z\d.-]+(?::\d+)?$/i.test(host)
        ? externalLink(
            `https://${host}/@${encodeURIComponent(node.props.username)}`,
            node.props.acct,
          )
        : inLink
          ? node.props.acct
          : text(node.props.acct);
    }
    case "hashtag":
      return inLink ? (
        `#${node.props.hashtag}`
      ) : (
        <ChannelHashtag name={node.props.hashtag} className={linkClass} />
      );
    case "search":
      return (
        <div className="mfm-search">
          {externalLink(
            `https://www.google.com/search?q=${encodeURIComponent(node.props.query)}`,
            <>⌕ {node.props.content}</>,
          )}
        </div>
      );
    case "fn": {
      const { name, args } = node.props;
      if (!(MFM_FUNCTIONS as readonly string[]).includes(name)) return toString(node);
      if (name === "ruby") {
        const last = node.children.at(-1);
        if (node.children.length === 1 && last?.type === "text") {
          const [base, ...reading] = last.props.text.split(" ");
          return (
            <ruby>
              {base}
              <rp>(</rp>
              <rt>{reading.join(" ")}</rt>
              <rp>)</rp>
            </ruby>
          );
        }
        return (
          <ruby>
            {render(node.children.slice(0, -1))}
            <rp>(</rp>
            <rt>{last?.type === "text" ? last.props.text.trim() : ""}</rt>
            <rp>)</rp>
          </ruby>
        );
      }
      if (name === "unixtime") {
        const raw = toString(node.children).trim();
        const timestamp = /^-?\d+$/.test(raw) ? Number(raw) * 1000 : NaN;
        const date = new Date(timestamp);
        if (!Number.isFinite(date.getTime())) return toString(node);
        const label = date.toLocaleString(undefined, { timeZoneName: "short" });
        return (
          <time dateTime={date.toISOString()} title={label} className="mfm-unixtime">
            {label}
          </time>
        );
      }
      if (name === "clickable")
        return inLink ? (
          children()
        ) : (
          <button
            type="button"
            className="mfm-clickable"
            onClick={(event) => {
              event.stopPropagation();
              const id = typeof args.ev === "string" ? args.ev : "";
              if (options.onClickEvent) options.onClickEvent(id);
              else
                event.currentTarget.dispatchEvent(
                  new CustomEvent("mfm-click", { detail: id, bubbles: true }),
                );
            }}
          >
            {children()}
          </button>
        );
      const { scale: nextScale, ...presentation } = functionPresentation(node.props, scale);
      const body = render(node.children, nextScale);
      return (
        <span {...presentation} tabIndex={name === "blur" ? 0 : undefined}>
          {name === "sparkle" ? (
            <>
              <span aria-hidden="true" className="mfm-sparkles">
                ✦ ✧
              </span>
              {body}
            </>
          ) : (
            body
          )}
        </span>
      );
    }
  }
  // Exhaustive check: new mfm-js AST types must be explicitly supported.
  const exhaustive: never = node;
  return exhaustive;
}

export function MfmContent({ content, ...options }: MfmContentProps) {
  const nodes = useMemo(() => parse(content, { nestLimit: 20 }), [content]);
  return (
    <>
      {nodes.map((node, index) => (
        <Fragment key={index}>{renderNode(node, options)}</Fragment>
      ))}
    </>
  );
}
