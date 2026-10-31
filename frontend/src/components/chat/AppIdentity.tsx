import { BotIcon } from "lucide-react";
import { Avatar, AvatarFallback, AvatarImage } from "../ui/avatar";
import { initials } from "./format";
import type { FeedMessage } from "./messages/messageFormat";
import { authorLabel } from "./messages/messageFormat";

export function openApp(id: string) {
  window.dispatchEvent(new CustomEvent("zoo:open-app", { detail: id }));
}
export function AppAvatar({
  label,
  isBot,
  src,
  size,
  className,
  title,
}: {
  label: string;
  isBot?: boolean;
  src?: string | null;
  size?: "sm" | "default";
  className?: string;
  title?: string;
}) {
  return (
    <Avatar size={size} className={className} title={title}>
      {src?.startsWith("https://") ? (
        <AvatarImage src={src} alt="" referrerPolicy="no-referrer" />
      ) : null}
      <AvatarFallback>
        {isBot ? <BotIcon className="size-4" aria-hidden="true" /> : initials(label)}
      </AvatarFallback>
    </Avatar>
  );
}
export function AuthorIdentity({ message }: { message: FeedMessage }) {
  const label = authorLabel(message);
  return (
    <span className="inline-flex min-w-0 items-center gap-1.5">
      {message.is_bot && message.app_id ? (
        <button
          type="button"
          className="truncate text-sm font-semibold hover:underline"
          onClick={() => openApp(message.app_id!)}
        >
          {label}
        </button>
      ) : (
        <span className="truncate text-sm font-semibold">{label}</span>
      )}
      {message.is_bot ? (
        <span className="rounded bg-muted px-1 py-0.5 text-[9px] font-semibold text-muted-foreground">
          APP
        </span>
      ) : null}
    </span>
  );
}
