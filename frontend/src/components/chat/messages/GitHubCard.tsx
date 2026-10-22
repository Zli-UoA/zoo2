import { GitForkIcon } from "lucide-react";

export const GITHUB_CARD_PREFIX = "zoo:github:v1\n";

export function parseGitHubCard(content: string) {
  if (!content.startsWith(GITHUB_CARD_PREFIX)) return null;
  try {
    const card: unknown = JSON.parse(content.slice(GITHUB_CARD_PREFIX.length));
    if (!card || typeof card !== "object") return null;
    const c = card as Record<string, unknown>;
    if (
      !["title", "url", "description", "repository", "actor", "event"].every(
        (key) => typeof c[key] === "string",
      )
    )
      return null;
    return c as Record<"title" | "url" | "description" | "repository" | "actor" | "event", string>;
  } catch {
    return null;
  }
}

export function GitHubCard({ card }: { card: NonNullable<ReturnType<typeof parseGitHubCard>> }) {
  let href: string | undefined;
  try {
    const url = new URL(card.url);
    if (url.protocol === "https:") href = url.href;
  } catch {
    /* A missing link still leaves a readable card. */
  }
  return (
    <article className="my-1 max-w-xl overflow-hidden rounded-lg border border-l-4 border-l-brand bg-card p-3 text-sm shadow-sm">
      <div className="mb-2 flex items-center gap-2 text-xs text-muted-foreground">
        <GitForkIcon className="size-4 shrink-0" aria-hidden="true" />
        <span className="break-all">GitHub · {card.repository}</span>
        <span className="ml-auto rounded border px-1.5 py-0.5">{card.event}</span>
      </div>
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
      <p className="mt-2 text-xs text-muted-foreground">{card.actor}</p>
    </article>
  );
}
