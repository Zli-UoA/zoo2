import { parse, toString, type MfmNode } from "mfm-js";
import katex from "katex";
import hljs from "highlight.js/lib/common";
import { functionPresentation, MFM_FUNCTIONS } from "../components/chat/messages/mfmFunctions";
import { accountEmojiNodes, messageBlocks } from "../components/chat/messages/mfmBlocks";
import "./renderer.css";

export interface MfmPayload {
  content: string;
  emojis: Record<string, string>;
  mentions: Record<string, boolean>;
  channels: Record<string, string>;
  restrictedMentions: boolean;
  dark: boolean;
  reducedMotion: boolean;
  active: boolean;
  fontSize: number;
}

let cachedContent: string | undefined;
let cachedBlocks: ReturnType<typeof messageBlocks> = [];

function blocks(content: string) {
  if (cachedContent !== content) {
    cachedBlocks = messageBlocks(accountEmojiNodes(parse(content, { nestLimit: 20 })));
    cachedContent = content;
  }
  return cachedBlocks;
}

export function emojiNames(content: string): string[] {
  const names = new Set<string>();
  const visit = (nodes: MfmNode[]) => {
    for (const node of nodes) {
      if (node.type === "emojiCode") names.add(`:${node.props.name}:`);
      else if (node.type === "text") {
        for (const match of node.props.text.matchAll(/(?=(:[^\s:]+:))/g)) names.add(match[1]);
      } else if (
        ["plain", "inlineCode", "blockCode", "url", "mathInline", "mathBlock"].includes(node.type)
      )
        continue;
      else if (
        node.type === "fn" &&
        !(MFM_FUNCTIONS as readonly string[]).includes(node.props.name)
      )
        continue;
      else if ("children" in node && node.children) visit(node.children);
    }
  };
  for (const block of blocks(content)) visit(block.nodes);
  return [...names].sort();
}

function element(tag: string, children: (Node | string)[] = []): HTMLElement {
  const result = document.createElement(tag);
  result.append(...children);
  return result;
}

export function safeURL(raw: string): string | null {
  try {
    const url = new URL(raw);
    return ["http:", "https:"].includes(url.protocol) &&
      !url.username &&
      !url.password &&
      url.hostname
      ? url.href
      : null;
  } catch {
    return null;
  }
}

function link(raw: string, children: (Node | string)[]): Node {
  const url = safeURL(raw);
  if (!url) return element("span", children);
  const result = element("a", children) as HTMLAnchorElement;
  result.href = url;
  result.rel = "noopener noreferrer";
  return result;
}

function emoji(code: string, payload: MfmPayload): Node | string {
  const value = Object.hasOwn(payload.emojis, code) ? payload.emojis[code] : code;
  if (!value.startsWith("data:image/png;base64,")) return value;
  const image = element("img") as HTMLImageElement;
  image.className = "mfm-custom-emoji";
  image.alt = code;
  image.src = value;
  image.addEventListener("error", () => image.replaceWith(code));
  return image;
}

function text(value: string, payload: MfmPayload): Node | string {
  // mfm-jsがtextとして残す日本語のスタンプ名も、登録済みのものだけ置換する。
  const result = document.createDocumentFragment();
  let cursor = 0;
  for (const match of value.matchAll(/(?=(:[^\s:]+:))/g)) {
    const code = match[1];
    if (match.index < cursor || !Object.hasOwn(payload.emojis, code)) continue;
    result.append(value.slice(cursor, match.index), emoji(code, payload));
    cursor = match.index + code.length;
  }
  if (cursor === 0) return value;
  result.append(value.slice(cursor));
  return result;
}

function renderNode(node: MfmNode, payload: MfmPayload, inLink = false, scale = 1): Node | string {
  const render = (nodes: MfmNode[], nextScale = scale, linked = inLink) =>
    nodes.map((child) => renderNode(child, payload, linked, nextScale));
  const children = () => render("children" in node ? (node.children ?? []) : []);
  switch (node.type) {
    case "text":
      return text(node.props.text, payload);
    case "unicodeEmoji":
      return node.props.emoji;
    case "emojiCode": {
      return emoji(`:${node.props.name}:`, payload);
    }
    case "bold":
      return element("strong", children());
    case "italic":
      return element("em", children());
    case "strike":
      return element("del", children());
    case "small":
      return element("small", children());
    case "inlineCode":
      return element("code", [node.props.code]);
    case "blockCode": {
      const code = element("code", [node.props.code]);
      if (node.props.lang && hljs.getLanguage(node.props.lang)) {
        code.className = "hljs";
        // エスケープ済みのHTMLを生成するライブラリだけにinnerHTMLを許可する。
        code.innerHTML = hljs.highlight(node.props.code, {
          language: node.props.lang,
          ignoreIllegals: true,
        }).value;
      }
      const block = element("pre", [code]);
      block.className = "mfm-code";
      return block;
    }
    case "plain":
      return toString(node.children);
    case "quote":
      return element("blockquote", children());
    case "center": {
      const result = element("div", children());
      result.style.textAlign = "center";
      return result;
    }
    case "mathInline":
    case "mathBlock": {
      const result = element("span");
      result.className = node.type === "mathBlock" ? "mfm-math-block" : "mfm-math";
      result.innerHTML = katex.renderToString(node.props.formula, {
        displayMode: node.type === "mathBlock",
        throwOnError: false,
        trust: false,
        strict: "ignore",
        maxExpand: 1000,
        maxSize: 20,
        output: "htmlAndMathml",
      });
      return result;
    }
    case "url":
      return inLink ? node.props.url : link(node.props.url, [node.props.url]);
    case "link":
      return inLink
        ? element("span", children())
        : safeURL(node.props.url)
          ? link(node.props.url, render(node.children, scale, true))
          : toString(node);
    case "mention": {
      if (inLink) return node.props.acct;
      if (node.props.host)
        return payload.restrictedMentions
          ? node.props.acct
          : link(`https://${node.props.host}/@${encodeURIComponent(node.props.username)}`, [
              node.props.acct,
            ]);
      const name = node.props.username;
      if (!Object.hasOwn(payload.mentions, name)) return node.props.acct;
      const result = element("span", [node.props.acct]);
      result.className = payload.mentions[name] ? "mention mention-self" : "mention";
      return result;
    }
    case "hashtag":
      return !inLink && Object.hasOwn(payload.channels, node.props.hashtag)
        ? link(payload.channels[node.props.hashtag], [`#${node.props.hashtag}`])
        : `#${node.props.hashtag}`;
    case "search":
      return link(`https://www.google.com/search?q=${encodeURIComponent(node.props.query)}`, [
        node.props.content,
      ]);
    case "fn": {
      const { name, args } = node.props;
      if (!(MFM_FUNCTIONS as readonly string[]).includes(name)) return toString(node);
      if (name === "unixtime") {
        const raw = toString(node.children).trim();
        const date = new Date(/^-?\d+$/.test(raw) ? Number(raw) * 1000 : NaN);
        if (!Number.isFinite(date.getTime())) return toString(node);
        const result = element("time", [date.toLocaleString(undefined, { timeZoneName: "short" })]);
        result.setAttribute("datetime", date.toISOString());
        return result;
      }
      if (name === "ruby") {
        const nodes = [...node.children];
        const last = nodes.at(-1);
        let reading = "";
        if (last?.type === "text") {
          const index = last.props.text.indexOf(" ");
          if (index >= 0) {
            reading = last.props.text.slice(index + 1).trim();
            nodes[nodes.length - 1] = {
              type: "text",
              props: { text: last.props.text.slice(0, index) },
            };
          }
        }
        return element("ruby", [
          ...render(nodes),
          element("rp", ["("]),
          element("rt", [reading]),
          element("rp", [")"]),
        ]);
      }
      if (name === "clickable") {
        const result = element(inLink ? "span" : "button", children());
        result.className = "mfm-clickable";
        result.addEventListener("click", () =>
          result.dispatchEvent(
            new CustomEvent("mfm-click", {
              detail: typeof args.ev === "string" ? args.ev : "",
              bubbles: true,
            }),
          ),
        );
        return result;
      }
      const presentation = functionPresentation(node.props, scale);
      const result = element("span", render(node.children, presentation.scale));
      result.className = presentation.className;
      // functionPresentationで検証した値だけをCSSプロパティへ渡す。
      for (const [key, value] of Object.entries(presentation.style)) {
        const property = key.replace(/[A-Z]/g, (letter) => `-${letter.toLowerCase()}`);
        const numeric = ["borderWidth", "borderRadius"].includes(key);
        result.style.setProperty(
          property,
          typeof value === "number" && numeric ? `${value}px` : String(value),
        );
      }
      if (name === "blur") {
        result.tabIndex = 0;
        result.setAttribute("role", "button");
        result.setAttribute("aria-label", "ぼかしを解除");
        const reveal = () => result.classList.toggle("revealed");
        result.addEventListener("click", reveal);
        result.addEventListener("keydown", (event) => {
          if (event.key === "Enter" || event.key === " ") {
            event.preventDefault();
            reveal();
          }
        });
      }
      if (name === "sparkle") {
        const stars = element("span", ["✦ ✧"]);
        stars.className = "mfm-sparkles";
        stars.setAttribute("aria-hidden", "true");
        result.prepend(stars);
      }
      return result;
    }
  }
  const exhaustive: never = node;
  return exhaustive;
}

export async function render(payload: MfmPayload): Promise<number> {
  const root = document.getElementById("content")!;
  document.documentElement.className = [
    payload.dark ? "dark" : "",
    payload.reducedMotion ? "reduce-motion" : "",
    payload.active ? "" : "paused",
  ].join(" ");
  document.body.style.fontSize = `${payload.fontSize}px`;
  const message = blocks(payload.content);
  const output = document.createDocumentFragment();
  message.forEach((block, index) => {
    const nodes = block.nodes.map((node) => renderNode(node, payload));
    if (block.level) {
      const heading = element(`h${block.level}`, nodes);
      heading.className = "mfm-heading";
      output.append(heading);
    } else output.append(...nodes);
    if (block.newline && !block.level && !message[index + 1]?.level) output.append("\n");
  });
  root.replaceChildren(output);
  await document.fonts.ready;
  await Promise.all(
    [...root.querySelectorAll("img")].map((image) => image.decode().catch(() => undefined)),
  );
  return Math.ceil(root.getBoundingClientRect().height);
}

Object.assign(window, { zooMfmRender: render, zooMfmEmojiNames: emojiNames });
