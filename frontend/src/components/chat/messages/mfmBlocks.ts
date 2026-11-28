import { toString, type MfmNode } from "mfm-js";

interface MessageBlock {
  nodes: MfmNode[];
  newline: boolean;
  level?: number;
}

// 見出しの判定はルートのテキストに限り、コード・plain・引用などの内部は変えない。
export function messageBlocks(nodes: MfmNode[]): MessageBlock[] {
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
      // mfm-jsがブロック前後の改行を消費する。
      flush();
      blocks.push({ nodes: [node], newline: false });
    } else {
      line.push(node);
    }
  }
  flush();
  return blocks;
}

/** :@name:をアカウント絵文字へまとめ、plainとリンクの内部は変えない。 */
export function accountEmojiNodes(input: MfmNode[]): MfmNode[] {
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
