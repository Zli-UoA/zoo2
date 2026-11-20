import { parse, type MfmNode } from "mfm-js";
import { messageTextSegments } from "./messageFormat";

/** Match visible MFM links, excluding code, plain text and silent links. */
export function previewUrls(content: string, excludeUrl?: string): string[] {
  const urls = new Set<string>();
  function add(raw: string) {
    try {
      const url = new URL(raw);
      if (
        /^https?:$/.test(url.protocol) &&
        !url.username &&
        !url.password &&
        raw.length <= 4096 &&
        url.href !== excludeUrl
      )
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
