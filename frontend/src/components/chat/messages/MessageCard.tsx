import { SquareArrowOutUpRightIcon } from "lucide-react";

/** Integration-neutral card schema. This data comes from the server-owned card column. */
export interface CardData {
  title: string;
  description: string;
  url: string;
  source: string;
  context: string;
  author: string;
  badge: string;
}
export function parseMessageCard(value?: string | null): CardData | null {
  if (!value) return null;
  try {
    const card: unknown = JSON.parse(value);
    if (!card || typeof card !== "object" || Array.isArray(card)) return null;
    const c = card as Record<string, unknown>;
    if (typeof c.title !== "string" || !c.title.trim() || c.title.length > 2000) return null;
    const result: CardData = {
      title: c.title,
      description: "",
      url: "",
      source: "",
      context: "",
      author: "",
      badge: "",
    };
    for (const key of ["description", "url", "source", "context", "author", "badge"] as const) {
      if (c[key] !== undefined && typeof c[key] !== "string") return null;
      result[key] = (c[key] as string | undefined) ?? "";
    }
    return result;
  } catch {
    return null;
  }
}
export function MessageCard({ card }: { card: CardData }) {
  let href: string | undefined;
  try {
    const url = new URL(card.url);
    if (url.protocol === "https:") href = url.href;
  } catch {
    /* Missing links render as text. */
  }
  return (
    <article className="my-1 max-w-xl overflow-hidden rounded-lg border border-l-4 border-l-brand bg-card p-3 text-sm shadow-sm">
      {card.source || card.context || card.badge ? (
        <div className="mb-2 flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
          <SquareArrowOutUpRightIcon className="size-4 shrink-0" aria-hidden="true" />
          <span className="min-w-0 break-all">
            {[card.source, card.context].filter(Boolean).join(" · ")}
          </span>
          {card.badge ? (
            <span className="ml-auto break-all rounded border px-1.5 py-0.5">{card.badge}</span>
          ) : null}
        </div>
      ) : null}
      <div className="break-words font-semibold">
        {href ? (
          <a
            href={href}
            target="_blank"
            rel="noopener noreferrer"
            className="text-brand underline-offset-2 hover:underline"
          >
            {card.title}
          </a>
        ) : (
          card.title
        )}
      </div>
      {card.description ? (
        <p className="mt-2 max-h-64 overflow-auto whitespace-pre-wrap break-words text-muted-foreground">
          {card.description}
        </p>
      ) : null}
      {card.author ? (
        <p className="mt-2 break-words text-xs text-muted-foreground">{card.author}</p>
      ) : null}
    </article>
  );
}
