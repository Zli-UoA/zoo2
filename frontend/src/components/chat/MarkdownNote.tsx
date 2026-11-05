import React from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import remarkParse from "remark-parse";
import { unified, type Plugin } from "unified";
import { CopyIcon } from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

type HastNode = {
  type: string;
  tagName?: string;
  value?: string;
  properties?: Record<string, unknown>;
  children?: HastNode[];
  position?: { start?: { offset?: number } };
};
type MdastNode = {
  type: string;
  checked?: boolean | null;
  value?: string;
  children?: MdastNode[];
  position?: { start?: { offset?: number } };
  data?: { hProperties?: Record<string, unknown> };
};

const taskParser = unified().use(remarkParse).use(remarkGfm);

function reactText(node: React.ReactNode): string {
  if (typeof node === "string" || typeof node === "number") return `${node}`;
  if (Array.isArray(node)) return node.map(reactText).join("");
  return "";
}

function taskMarkerPositions(tree: MdastNode, content: string): Map<MdastNode, number> {
  const positions = new Map<MdastNode, number>();
  const visit = (node: MdastNode) => {
    if (node.type === "listItem") {
      const start = node.position?.start?.offset;
      const paragraph = node.children?.find((child) => child.type === "paragraph");
      const paragraphStart = paragraph?.position?.start?.offset;
      const firstText = paragraph?.children?.[0];
      let stateOffset: number | undefined;
      if (
        (node.checked === null || node.checked === undefined) &&
        firstText?.type === "text" &&
        paragraph?.children?.length === 1 &&
        /^\[[ xX]\]$/.test(firstText.value ?? "") &&
        paragraphStart !== undefined &&
        content.slice(paragraphStart, paragraphStart + 3) === firstText.value
      ) {
        node.checked = firstText.value?.[1]?.toLowerCase() === "x";
        firstText.value = "";
        stateOffset = paragraphStart + 1;
      } else if (
        node.checked !== null &&
        node.checked !== undefined &&
        start !== undefined &&
        paragraphStart !== undefined
      ) {
        const prefix = content.slice(start, paragraphStart);
        const marker = /\[([ xX])\]/g;
        let match: RegExpExecArray | null;
        while ((match = marker.exec(prefix))) stateOffset = start + match.index + 1;
      }
      if (stateOffset !== undefined) positions.set(node, stateOffset);
    }
    node.children?.forEach(visit);
  };
  visit(tree);
  return positions;
}

export function toggleTaskMarker(content: string, sourceOffset: number, checked: boolean): string {
  if (!Number.isInteger(sourceOffset) || sourceOffset < 0 || sourceOffset >= content.length)
    return content;
  const tree = taskParser.runSync(taskParser.parse(content)) as MdastNode;
  const markerPositions = taskMarkerPositions(tree, content);
  if (!Array.from(markerPositions.values()).includes(sourceOffset)) return content;
  const nextMarker = checked ? "x" : " ";
  if (content[sourceOffset] === nextMarker) return content;
  return content.slice(0, sourceOffset) + nextMarker + content.slice(sourceOffset + 1);
}

function safeHref(raw: string): string {
  const value = raw.trim();
  return /^(https?:|mailto:)/i.test(value) || /^(\/(?!\/)|\.\.?\/|#)/.test(value) ? value : "";
}

function rehypeMentions() {
  return (tree: HastNode) => {
    const visit = (parent: HastNode) => {
      if (parent.tagName === "code" || parent.tagName === "pre") return;
      if (!parent.children) return;
      parent.children = parent.children.flatMap((node) => {
        if (node.type === "text" && typeof node.value === "string") {
          const result: HastNode[] = [];
          const pattern = /@[A-Za-z0-9._-]+/g;
          let offset = 0;
          for (const match of node.value.matchAll(pattern)) {
            const index = match.index ?? 0;
            const previous = node.value[index - 1];
            if (previous && !/[\s([{<（「『"'。、]/.test(previous)) continue;
            if (index > offset)
              result.push({ type: "text", value: node.value.slice(offset, index) });
            result.push({
              type: "element",
              tagName: "span",
              properties: { className: ["font-medium", "text-brand"] },
              children: [{ type: "text", value: match[0] }],
            });
            offset = index + match[0].length;
          }
          if (offset === 0) return [node];
          if (offset < node.value.length)
            result.push({ type: "text", value: node.value.slice(offset) });
          return result;
        }
        visit(node);
        return [node];
      });
    };
    visit(tree);
  };
}

const remarkTaskPositions: Plugin = () => (inputTree, file) => {
  const tree = inputTree as MdastNode;
  const content =
    typeof file.value === "string" ? file.value : new TextDecoder().decode(file.value);
  const markers = taskMarkerPositions(tree, content);
  const visit = (node: MdastNode) => {
    const markerOffset = markers.get(node);
    if (markerOffset !== undefined) {
      node.data ??= {};
      node.data.hProperties = {
        ...node.data.hProperties,
        "data-task-marker-offset": markerOffset,
      };
    }
    node.children?.forEach(visit);
  };
  visit(tree);
};

function rehypeTaskPositions() {
  return (tree: HastNode) => {
    const textContent = (node: HastNode): string => {
      if (node.tagName === "input" || node.tagName === "ul" || node.tagName === "ol") return "";
      return (node.value ?? "") + (node.children?.map(textContent).join("") ?? "");
    };
    const visit = (node: HastNode) => {
      if (node.tagName === "li" && node.position && node.children) {
        const checkboxes = node.children.flatMap((child) =>
          child.tagName === "input"
            ? [child]
            : child.tagName === "p"
              ? (child.children ?? []).filter(
                  (paragraphChild) => paragraphChild.tagName === "input",
                )
              : [],
        );
        for (const checkbox of checkboxes) {
          if (checkbox.properties?.type !== "checkbox") continue;
          const markerOffset = node.properties?.["data-task-marker-offset"];
          if (typeof markerOffset === "number") {
            checkbox.position = { ...node.position, start: { offset: markerOffset } };
          } else {
            checkbox.position = undefined;
          }
          const label = node.children.map(textContent).join("").trim();
          checkbox.properties = {
            ...checkbox.properties,
            "aria-label": label || "Task checkbox",
          };
        }
      }
      node.children?.forEach(visit);
    };
    visit(tree);
  };
}

/** GFM renderer with raw HTML disabled and an explicit safe URL allowlist. */
export function MarkdownNote({
  content,
  taskCheckboxes = false,
  onTaskToggle,
}: {
  content: string;
  taskCheckboxes?: boolean;
  onTaskToggle?: (sourceOffset: number, checked: boolean) => void;
}) {
  const { t } = useTranslation();
  async function copyCode(value: string) {
    try {
      await navigator.clipboard.writeText(value);
      toast.success(t("channels.notes.copied"));
    } catch {
      toast.error(t("channels.notes.copyError"));
    }
  }
  return (
    <div className="markdown-note max-w-none break-words">
      <ReactMarkdown
        remarkPlugins={[remarkGfm, remarkTaskPositions]}
        rehypePlugins={[rehypeMentions, rehypeTaskPositions]}
        urlTransform={safeHref}
        components={{
          li: ({ node, children, ...props }) => {
            const rawClass = (node as unknown as HastNode | undefined)?.properties?.className;
            const task = Array.isArray(rawClass)
              ? rawClass.includes("task-list-item")
              : typeof rawClass === "string" && rawClass.includes("task-list-item");
            return (
              <li {...props} className={task ? "task-list-item" : props.className}>
                {children}
              </li>
            );
          },
          pre: ({ children, ...props }) => {
            const code = React.Children.toArray(children).find(
              (child): child is React.ReactElement<{ children?: React.ReactNode }> =>
                React.isValidElement(child),
            );
            const source = code ? reactText(code.props.children) : "";
            return (
              <div className="markdown-code-block">
                <button
                  type="button"
                  className="markdown-code-copy"
                  aria-label={t("channels.notes.copyCode")}
                  onClick={() => void copyCode(source)}
                >
                  <CopyIcon aria-hidden="true" />
                </button>
                <pre {...props}>{children}</pre>
              </div>
            );
          },
          input: ({ node, checked, ...props }) => {
            const sourceOffset = node?.position?.start?.offset;
            return (
              <input
                {...props}
                type="checkbox"
                checked={Boolean(checked)}
                disabled={!taskCheckboxes || sourceOffset === undefined}
                onChange={(event) => {
                  if (sourceOffset !== undefined)
                    onTaskToggle?.(sourceOffset, event.currentTarget.checked);
                }}
              />
            );
          },
          a: ({ children, ...props }) => (
            <a {...props} rel="noreferrer noopener" className="text-brand underline">
              {children}
            </a>
          ),
        }}
      >
        {content}
      </ReactMarkdown>
    </div>
  );
}
