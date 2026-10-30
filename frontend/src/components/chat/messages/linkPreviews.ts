import { parse, type MfmNode } from "mfm-js";
import { messageTextSegments } from "./messageFormat";

/** Match visible MFM links, excluding code, plain text and silent links. */
export function previewUrls(content: string): string[] {
  const urls = new Set<string>();
  function add(raw: string) {
    try {
      const url = new URL(raw);
      if (/^https?:$/.test(url.protocol) && !url.username && !url.password && raw.length <= 4096)
        urls.add(url.href);
    } catch {
      /* Incomplete URLs remain text. */
    }
  }
  function walk(nodes: MfmNode[]) {
    for (const node of nodes) {
      if (node.type === "url") add(node.props.url);
      else if (node.type === "link") {
        if (!node.props.silent) add(node.props.url);
      } else if (node.type === "text") {
        for (const part of messageTextSegments(node.props.text))
          if (part.kind === "link") add(part.value);
      } else if (node.type !== "plain" && "children" in node) walk(node.children ?? []);
    }
  }
  walk(parse(content));
  return [...urls].slice(0, 3);
}

export function proxyImageUrl(endpoint: string, image: string | null): string | null {
  if (!endpoint || !image) return null;
  try {
    const source = new URL(image);
    if (!/^https?:$/.test(source.protocol) || source.username || source.password) return null;
    // Relative endpoints must stay on this origin; protocol-relative URLs are rejected.
    const relative = endpoint.startsWith("/") && !endpoint.startsWith("//");
    const proxy = new URL(endpoint, relative ? "https://same-origin.invalid" : undefined);
    if (
      proxy.protocol !== "https:" &&
      !(proxy.protocol === "http:" && /^(localhost|127\.0\.0\.1)$/.test(proxy.hostname))
    )
      return null;
    if (proxy.username || proxy.password) return null;
    proxy.searchParams.set("url", source.href);
    proxy.searchParams.set("preview", "1");
    proxy.searchParams.set("static", "1");
    return relative ? `${proxy.pathname}${proxy.search}` : proxy.href;
  } catch {
    return null;
  }
}
