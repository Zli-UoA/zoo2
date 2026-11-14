import {
  createContext,
  useContext,
  createElement,
  Fragment,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import { parse, toString, type MfmNode } from "mfm-js";
import katex from "katex";
import hljs from "highlight.js/lib/common";
import { useWorkspaceEmojis } from "../../../lib/emoji/workspace";
import type { DisplayEmoji } from "../../../lib/electric/types";
import { emojiImageStyle } from "../../../lib/emoji/dimensions";
import { useShortcodes } from "../../../lib/emoji/shortcodes";
import { ExternalLink, inlineLinkClassName } from "../../ui/external-link";
import { MfmSearch } from "./MfmSearch";
import { ChannelHashtag } from "./MfmChannels";
import { AccountGlyph } from "./MfmAccounts";
import { messageTextSegments } from "./messageFormat";
import { isAllowedMention, type MentionAllowlist } from "./mentions";
import { functionPresentation, MFM_FUNCTIONS } from "./mfmFunctions";

export interface MfmContentProps {
  content: string;
  /** Registered custom emoji: name -> same-origin image URL. Unknown names stay readable. */
  emojiUrls?: Readonly<Record<string, string>>;
  /** Joined into the message by the server; an empty object is authoritative. */
  emojis?: Readonly<Record<string, DisplayEmoji>>;
  /** Trusted renderer for registered emoji, including proxied federation images. */
  renderEmoji?: (name: string) => ReactNode;
  /** Application event, never evaluated as JavaScript. */
  onClickEvent?: (event: string) => void;
  /**
   * 暗号化メッセージで、メンションとして表示してよい `@name`（復号した `mentions` から作る）。
   * 渡したときは、ここにない `@name` を素の文字列で出し、外部のアカウントへのリンクも作らない。
   * 省くと（平文のメッセージ）、今までどおり本文の `@name` をすべてメンションとして表示する。
   */
  mentions?: MentionAllowlist;
}
const ShortcodeContext = createContext<Readonly<Record<string, string>>>({});
const linkClass = `${inlineLinkClassName} underline`;
function externalLink(url: string, children: ReactNode): ReactNode {
  return (
    <ExternalLink href={url} className="underline [overflow-wrap:anywhere]">
      {children}
    </ExternalLink>
  );
}
function text(content: string, mentions?: MentionAllowlist): ReactNode {
  return messageTextSegments(content).map((segment, index) => (
    <Fragment key={index}>
      {segment.kind === "link" ? (
        externalLink(segment.value, segment.value.replace(/^https?:\/\//i, ""))
      ) : segment.kind === "mention" && (!mentions || isAllowedMention(segment.value, mentions)) ? (
        <span className="rounded bg-brand-muted px-0.5 font-medium text-brand-muted-foreground">
          <AccountGlyph name={segment.value.slice(1)} mention />
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
  emojis,
}: {
  name: string;
  emojiUrls?: MfmContentProps["emojiUrls"];
  emojis?: MfmContentProps["emojis"];
}) {
  const shortcodes = useContext(ShortcodeContext);
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
  if (url && (/^\/(?!\/)/.test(url) || /^https?:\/\//.test(url)))
    return (
      <img
        className="mfm-custom-emoji"
        style={emojiImageStyle(emojis?.[name], "1.5em")}
        src={url}
        alt={`:${name}:`}
        loading="lazy"
      />
    );
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
      return inLink ? node.props.text : text(node.props.text, options.mentions);
    case "unicodeEmoji":
      return node.props.emoji;
    case "emojiCode":
      if (node.props.name.startsWith("@")) return <AccountGlyph name={node.props.name.slice(1)} />;
      return (
        options.renderEmoji?.(node.props.name) ?? (
          <EmojiCode name={node.props.name} emojiUrls={options.emojiUrls} emojis={options.emojis} />
        )
      );
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
      return inLink ? node.props.url : text(node.props.url, options.mentions);
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
      // 暗号化メッセージの `mentions` はこのワークスペースのユーザー ID だけなので、外部のアカウントは
      // メンションとして扱わず、素の文字列で出す。
      if (host && options.mentions) return node.props.acct;
      return !inLink && host && /^[a-z\d.-]+(?::\d+)?$/i.test(host)
        ? externalLink(
            `https://${host}/@${encodeURIComponent(node.props.username)}`,
            node.props.acct,
          )
        : inLink
          ? node.props.acct
          : text(node.props.acct, options.mentions);
    }
    case "hashtag":
      return inLink ? (
        `#${node.props.hashtag}`
      ) : (
        <ChannelHashtag name={node.props.hashtag} className={linkClass} />
      );
    case "search":
      return inLink ? node.props.content : <MfmSearch query={node.props.query} />;
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

interface MessageBlock {
  nodes: MfmNode[];
  newline: boolean;
  level?: number;
}

// Only inspect root text. Code, plain text, quotes and other MFM subtrees stay intact.
function messageBlocks(nodes: MfmNode[]): MessageBlock[] {
  const blocks: MessageBlock[] = [];
  let line: MfmNode[] = [];
  const flush = (newline = false) => {
    const first = line[0];
    const marker = first?.type === "text" ? /^ {0,3}(#{1,6})[ \t]+/.exec(first.props.text) : null;
    let level: number | undefined;
    if (marker && toString(line).slice(marker[0].length).trim()) {
      level = marker[1].length;
      line[0] = {
        type: "text",
        props: { text: (first?.type === "text" ? first.props.text : "").slice(marker[0].length) },
      };
      const last = line.at(-1);
      if (last?.type === "text") {
        line[line.length - 1] = {
          type: "text",
          props: { text: last.props.text.replace(/[ \t]+#+[ \t]*$/, "").trimEnd() },
        };
      }
    }
    if (line.length || newline) blocks.push({ nodes: line, newline, level });
    line = [];
  };
  for (const node of nodes) {
    if (node.type === "text") {
      node.props.text.split("\n").forEach((part, index) => {
        if (index) flush(true);
        line.push({ type: "text", props: { text: part } });
      });
    } else if (["blockCode", "quote", "center", "mathBlock", "search"].includes(node.type)) {
      // mfm-js consumes the line breaks around block nodes.
      flush();
      blocks.push({ nodes: [node], newline: false });
    } else {
      line.push(node);
    }
  }
  flush();
  return blocks;
}

export function MfmContent({ content, ...options }: MfmContentProps) {
  const shortcodes = useShortcodes(/:[a-zA-Z0-9_+-]+:/.test(content));
  const workspace = useWorkspaceEmojis(
    options.emojis === undefined && /:[a-z0-9_-]+:/.test(content),
  );
  const emojiUrls = Object.fromEntries(
    workspace.emojis.flatMap((e) =>
      [e.name, ...(e.aliases ?? [])].map((name) => [name, workspace.imageUrl(e.url)]),
    ),
  );
  const resolvedOptions = {
    ...options,
    emojiUrls:
      options.emojis !== undefined || workspace.emojis.length || options.emojiUrls
        ? {
            ...emojiUrls,
            ...Object.fromEntries(
              Object.entries(options.emojis ?? {}).map(([name, emoji]) => [
                name,
                workspace.imageUrl(emoji.url),
              ]),
            ),
            ...options.emojiUrls,
          }
        : undefined,
  };
  const blocks = useMemo(
    () => messageBlocks(accountEmojiNodes(parse(content, { nestLimit: 20 }))),
    [content],
  );
  return (
    <ShortcodeContext.Provider value={shortcodes}>
      {blocks.map((block, index) => {
        const body = block.nodes.map((node, childIndex) => (
          <Fragment key={childIndex}>{renderNode(node, resolvedOptions)}</Fragment>
        ));
        return (
          <Fragment key={index}>
            {block.level
              ? createElement(`h${block.level}`, { className: "mfm-heading" }, body)
              : body}
            {block.newline && !block.level && !blocks[index + 1]?.level ? "\n" : null}
          </Fragment>
        );
      })}
    </ShortcodeContext.Provider>
  );
}

/** mfm-js parses :@name: as text + mention + text. Preserve protected subtrees. */
function accountEmojiNodes(input: MfmNode[]): MfmNode[] {
  const nodes = input.map((node) => {
    if ("children" in node && !["plain", "link"].includes(node.type))
      return { ...node, children: accountEmojiNodes(node.children ?? []) } as MfmNode;
    return node;
  });
  const out: MfmNode[] = [];
  for (let i = 0; i < nodes.length; i++) {
    const node = nodes[i];
    const previous = out.at(-1);
    const next = nodes[i + 1];
    if (
      node.type === "mention" &&
      !node.props.host &&
      previous?.type === "text" &&
      previous.props.text.endsWith(":") &&
      next?.type === "text" &&
      next.props.text.startsWith(":")
    ) {
      out[out.length - 1] = { type: "text", props: { text: previous.props.text.slice(0, -1) } };
      out.push({ type: "emojiCode", props: { name: `@${node.props.username}` } });
      nodes[i + 1] = { type: "text", props: { text: next.props.text.slice(1) } };
    } else out.push(node);
  }
  return out;
}
