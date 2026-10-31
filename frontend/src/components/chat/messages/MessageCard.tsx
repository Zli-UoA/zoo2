import { ExternalLink } from "../../ui/external-link";
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
  color?: string;
  fields?: { title: string; value: string; short?: boolean; url?: string; code?: boolean }[];
}

function cardColor(value: unknown): string | undefined {
  if (typeof value !== "string") return undefined;
  const named: Record<string, string> = { good: "#2eb886", warning: "#daa038", danger: "#a30200" };
  if (Object.hasOwn(named, value)) return named[value];
  return /^#?[0-9a-fA-F]{6}$/.test(value) ? `#${value.replace(/^#/, "")}` : undefined;
}

export function parseMessageCards(value?: string | null): CardData[] | null {
  if (!value) return null;
  try {
    const parsed: unknown = JSON.parse(value);
    if (parsed && typeof parsed === "object" && "cards" in parsed) {
      if (!Array.isArray(parsed.cards) || !parsed.cards.length || parsed.cards.length > 101)
        return null;
      const cards = parsed.cards.map((card: unknown) => parseMessageCard(JSON.stringify(card)));
      return cards.every((card): card is CardData => card !== null) ? cards : null;
    }
    const card = parseMessageCard(value);
    return card ? [card] : null;
  } catch {
    return null;
  }
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
    const color = cardColor(c.color);
    if (color) result.color = color;
    if (c.fields !== undefined) {
      if (!Array.isArray(c.fields) || c.fields.length > 10) return null;
      const fields: NonNullable<CardData["fields"]> = [];
      for (const field of c.fields) {
        if (
          !field ||
          typeof field !== "object" ||
          typeof field.title !== "string" ||
          field.title.length > 100 ||
          typeof field.value !== "string" ||
          field.value.length > 4000 ||
          (field.url !== undefined && typeof field.url !== "string") ||
          (field.short !== undefined && typeof field.short !== "boolean") ||
          (field.code !== undefined && typeof field.code !== "boolean")
        )
          return null;
        fields.push(field);
      }
      result.fields = fields;
    }
    return result;
  } catch {
    return null;
  }
}
function safeHref(value?: string): string | undefined {
  try {
    const url = new URL(value ?? "");
    if (url.protocol === "https:") return url.href;
  } catch {
    /* Plain text fallback. */
  }
}
export function MessageCard({
  card,
  attachment = false,
}: {
  card: CardData;
  attachment?: boolean;
}) {
  const href = safeHref(card.url);
  return (
    <article
      className={
        attachment
          ? "@container my-2 max-w-2xl overflow-hidden rounded-sm border-l-4 border-l-brand py-1 pl-4 pr-2 text-sm"
          : "@container my-1 max-w-xl overflow-hidden rounded-lg border border-l-4 border-l-brand bg-card p-3 text-sm shadow-sm"
      }
      style={cardColor(card.color) ? { borderLeftColor: cardColor(card.color) } : undefined}
    >
      {!attachment && (card.source || card.context || card.badge) ? (
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
          <ExternalLink href={href} className="hover:underline">
            {card.title}
          </ExternalLink>
        ) : (
          card.title
        )}
      </div>
      {card.description ? (
        <p className="mt-2 max-h-64 overflow-auto whitespace-pre-wrap break-words text-muted-foreground">
          {card.description}
        </p>
      ) : null}
      {card.fields?.length ? (
        <dl className="mt-3 grid grid-cols-1 gap-x-6 gap-y-3 @sm:grid-cols-2">
          {card.fields.map((field, index) => {
            const href = safeHref(field.url);
            const value = field.code ? (
              <code className="rounded border bg-muted px-1 py-0.5 font-mono text-xs break-all">
                {field.value}
              </code>
            ) : (
              field.value
            );
            return (
              <div key={index} className={field.short ? "min-w-0" : "min-w-0 @sm:col-span-2"}>
                <dt className="mb-1 font-semibold">{field.title}</dt>
                <dd className="whitespace-pre-wrap break-words">
                  {href ? (
                    <ExternalLink href={href} className="hover:underline">
                      {value}
                    </ExternalLink>
                  ) : (
                    value
                  )}
                </dd>
              </div>
            );
          })}
        </dl>
      ) : null}
      {attachment && (card.source || card.context || card.badge) ? (
        <div className="mt-3 flex flex-wrap items-center gap-1.5 text-xs text-muted-foreground">
          <SquareArrowOutUpRightIcon className="size-3.5 shrink-0" aria-hidden="true" />
          <span className="break-all">
            {[card.source, card.context, card.badge].filter(Boolean).join(" · ")}
          </span>
        </div>
      ) : null}
      {card.author ? (
        <p className="mt-2 break-words text-xs text-muted-foreground">{card.author}</p>
      ) : null}
    </article>
  );
}
