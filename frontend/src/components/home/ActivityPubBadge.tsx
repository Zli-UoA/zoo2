// ActivityPub logo: Simple Icons, CC0. https://simpleicons.org/
import { useTranslation } from "react-i18next";
export function ActivityPubBadge({ visibility }: { visibility: string }) {
  const { t } = useTranslation();
  const audience =
    visibility === "unlisted" || visibility === "followers" || visibility === "local"
      ? visibility
      : "public";
  return (
    <span
      className="inline-flex shrink-0 items-center gap-1 text-xs text-muted-foreground"
      title={`ActivityPub · ${t(`home.visibility.${audience}`)}`}
    >
      <svg viewBox="0 0 24 24" className="size-4" role="img" aria-label="ActivityPub">
        <path
          fill="currentColor"
          d="M10.91 4.442L0 10.74v2.52L8.727 8.22v10.077l2.182 1.26zM6.545 12l-4.364 2.52 4.364 2.518zm6.545-2.52L17.455 12l-4.364 2.52zm0-5.038L24 10.74v2.52l-10.91 6.298v-2.52L21.819 12 13.091 6.96z"
        />
      </svg>
      <span>{t(`home.visibility.${audience}`)}</span>
    </span>
  );
}
