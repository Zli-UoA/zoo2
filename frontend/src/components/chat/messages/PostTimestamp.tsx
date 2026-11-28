import { useTranslation } from "react-i18next";
import { useLocalDay } from "../../../hooks/useLocalDay";
import { DEFAULT_LOCALE, matchLocale } from "../../../lib/i18n/locale";
import { normalizeUtcTimestamp } from "../../../lib/timestamps";
import { formatTime, fullTimestamp } from "./messageFormat";

export function usePostTimestamp(createdAt: string) {
  const today = useLocalDay();
  const { i18n } = useTranslation();
  const locale = matchLocale(i18n.language ?? DEFAULT_LOCALE) ?? DEFAULT_LOCALE;
  return {
    today,
    dateTime: normalizeUtcTimestamp(createdAt),
    title: fullTimestamp(createdAt, locale),
    label: formatTime(createdAt, locale, today),
  };
}

export function PostTimestamp({ createdAt, className }: { createdAt: string; className?: string }) {
  const { dateTime, title, label } = usePostTimestamp(createdAt);
  return (
    <time className={className} dateTime={dateTime} title={title}>
      {label}
    </time>
  );
}
